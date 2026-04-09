//! Codebase context detection — auto-discovers project characteristics
//! for smarter routing decisions.
//!
//! Scans the working directory for: languages, file count, test frameworks,
//! build tools. Results are cached in `.codex-multi/context_cache.json`
//! and injected into the classifier prompt.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::info;

/// Auto-detected codebase context.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CodebaseContext {
    pub languages: Vec<String>,
    pub file_count: usize,
    pub test_frameworks: Vec<String>,
    pub build_tools: Vec<String>,
    pub has_docker: bool,
    pub has_ci: bool,
    pub estimated_complexity: String,  // "small", "medium", "large"
}

impl CodebaseContext {
    /// Detect codebase context from the working directory.
    /// Uses cache if available and fresh (< 1 hour).
    pub fn detect(project_dir: &Path) -> Self {
        let cache_path = project_dir
            .join(".codex-multi")
            .join("context_cache.json");

        // Check cache freshness
        if let Ok(metadata) = std::fs::metadata(&cache_path) {
            if let Ok(modified) = metadata.modified() {
                if modified.elapsed().unwrap_or_default().as_secs() < 3600 {
                    if let Ok(content) = std::fs::read_to_string(&cache_path) {
                        if let Ok(ctx) = serde_json::from_str::<CodebaseContext>(&content) {
                            return ctx;
                        }
                    }
                }
            }
        }

        // Scan and cache
        let ctx = scan_directory(project_dir);

        if let Some(parent) = cache_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(&ctx) {
            let _ = std::fs::write(&cache_path, json);
        }

        info!(
            languages = ?ctx.languages,
            files = ctx.file_count,
            complexity = %ctx.estimated_complexity,
            "Detected codebase context"
        );

        ctx
    }

    /// Format for injection into the classifier prompt.
    pub fn classifier_context(&self) -> String {
        if self.languages.is_empty() {
            return String::new();
        }
        let mut parts = vec![format!(
            "Project: {} files, {} complexity",
            self.file_count, self.estimated_complexity
        )];
        parts.push(format!("Languages: {}", self.languages.join(", ")));
        if !self.test_frameworks.is_empty() {
            parts.push(format!("Tests: {}", self.test_frameworks.join(", ")));
        }
        if !self.build_tools.is_empty() {
            parts.push(format!("Build: {}", self.build_tools.join(", ")));
        }
        parts.join(". ")
    }
}

fn scan_directory(dir: &Path) -> CodebaseContext {
    let mut extensions: HashMap<String, usize> = HashMap::new();
    let mut file_count = 0;
    let mut test_frameworks = Vec::new();
    let mut build_tools = Vec::new();
    let mut has_docker = false;
    let mut has_ci = false;

    // Walk the directory (max depth 4 to avoid huge repos)
    scan_recursive(dir, &mut extensions, &mut file_count, &mut test_frameworks,
                   &mut build_tools, &mut has_docker, &mut has_ci, 0, 4);

    // Map extensions to language names
    let ext_to_lang: HashMap<&str, &str> = HashMap::from([
        ("py", "Python"), ("js", "JavaScript"), ("ts", "TypeScript"),
        ("tsx", "TypeScript"), ("jsx", "JavaScript"), ("rs", "Rust"),
        ("go", "Go"), ("java", "Java"), ("rb", "Ruby"), ("php", "PHP"),
        ("cpp", "C++"), ("c", "C"), ("cs", "C#"), ("swift", "Swift"),
        ("kt", "Kotlin"), ("sql", "SQL"), ("sh", "Shell"),
        ("html", "HTML"), ("css", "CSS"), ("vue", "Vue"), ("svelte", "Svelte"),
    ]);

    let mut lang_counts: HashMap<String, usize> = HashMap::new();
    for (ext, count) in &extensions {
        if let Some(lang) = ext_to_lang.get(ext.as_str()) {
            *lang_counts.entry(lang.to_string()).or_default() += count;
        }
    }

    let mut languages: Vec<(String, usize)> = lang_counts.into_iter().collect();
    languages.sort_by(|a, b| b.1.cmp(&a.1));
    let languages: Vec<String> = languages.into_iter().map(|(l, _)| l).collect();

    test_frameworks.dedup();
    build_tools.dedup();

    let estimated_complexity = if file_count < 50 {
        "small"
    } else if file_count < 500 {
        "medium"
    } else {
        "large"
    }
    .to_string();

    CodebaseContext {
        languages,
        file_count,
        test_frameworks,
        build_tools,
        has_docker,
        has_ci,
        estimated_complexity,
    }
}

fn scan_recursive(
    dir: &Path,
    extensions: &mut HashMap<String, usize>,
    file_count: &mut usize,
    test_frameworks: &mut Vec<String>,
    build_tools: &mut Vec<String>,
    has_docker: &mut bool,
    has_ci: &mut bool,
    depth: usize,
    max_depth: usize,
) {
    if depth > max_depth {
        return;
    }

    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip hidden dirs and common non-source dirs
        if name.starts_with('.') || name == "node_modules" || name == "target"
            || name == "__pycache__" || name == "venv" || name == ".venv"
            || name == "dist" || name == "build"
        {
            continue;
        }

        if path.is_dir() {
            scan_recursive(
                &path, extensions, file_count, test_frameworks,
                build_tools, has_docker, has_ci, depth + 1, max_depth,
            );
            continue;
        }

        *file_count += 1;

        // Count by extension
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            *extensions.entry(ext.to_lowercase()).or_default() += 1;
        }

        // Detect test frameworks
        match name.as_str() {
            "pytest.ini" | "conftest.py" | "pyproject.toml" => {
                if !test_frameworks.contains(&"pytest".to_string()) {
                    test_frameworks.push("pytest".to_string());
                }
            }
            "jest.config.js" | "jest.config.ts" => {
                if !test_frameworks.contains(&"jest".to_string()) {
                    test_frameworks.push("jest".to_string());
                }
            }
            "playwright.config.ts" | "playwright.config.js" => {
                if !test_frameworks.contains(&"playwright".to_string()) {
                    test_frameworks.push("playwright".to_string());
                }
            }
            "vitest.config.ts" => {
                if !test_frameworks.contains(&"vitest".to_string()) {
                    test_frameworks.push("vitest".to_string());
                }
            }
            _ => {}
        }

        // Detect build tools
        match name.as_str() {
            "package.json" => {
                if !build_tools.contains(&"npm".to_string()) {
                    build_tools.push("npm".to_string());
                }
            }
            "Cargo.toml" => {
                if !build_tools.contains(&"cargo".to_string()) {
                    build_tools.push("cargo".to_string());
                }
            }
            "Makefile" | "makefile" => {
                if !build_tools.contains(&"make".to_string()) {
                    build_tools.push("make".to_string());
                }
            }
            "go.mod" => {
                if !build_tools.contains(&"go".to_string()) {
                    build_tools.push("go".to_string());
                }
            }
            _ => {}
        }

        // Detect Docker and CI
        if name.starts_with("Dockerfile") || name == "docker-compose.yml" || name == "docker-compose.yaml" {
            *has_docker = true;
        }
        if name == ".github" || name == ".gitlab-ci.yml" || name == "Jenkinsfile" {
            *has_ci = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_creates_context() {
        let dir = std::env::temp_dir().join("codebase_ctx_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/main.py"), "print('hello')").unwrap();
        std::fs::write(dir.join("src/utils.py"), "pass").unwrap();
        std::fs::write(dir.join("conftest.py"), "").unwrap();

        let ctx = CodebaseContext::detect(&dir);
        assert!(ctx.languages.contains(&"Python".to_string()));
        assert!(ctx.file_count >= 3);
        assert!(ctx.test_frameworks.contains(&"pytest".to_string()));
        assert_eq!(ctx.estimated_complexity, "small");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_classifier_context_format() {
        let ctx = CodebaseContext {
            languages: vec!["Python".into(), "TypeScript".into()],
            file_count: 250,
            test_frameworks: vec!["pytest".into(), "playwright".into()],
            build_tools: vec!["npm".into()],
            has_docker: true,
            has_ci: true,
            estimated_complexity: "medium".into(),
        };
        let s = ctx.classifier_context();
        assert!(s.contains("250 files"));
        assert!(s.contains("medium"));
        assert!(s.contains("Python"));
        assert!(s.contains("pytest"));
    }
}

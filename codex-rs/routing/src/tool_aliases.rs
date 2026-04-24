//! Translate hallucinated tool names into proper `shell` invocations.
//!
//! Small local models (notably qwen3.5:9b) habitually emit shell command
//! names — `ls`, `rg`, `cat`, `git`, `pytest`, etc. — as tool names. They
//! pattern-match on their training data, where shell access typically appears
//! under those names directly. The translation below catches that and rewrites
//! the call into a real `shell` invocation so Codex's tool registry can run
//! it. Same set runs in regular and local-only modes.
//!
//! Why this isn't a "band-aid" by AGENTS.md's definition: the upstream fix
//! is the model's training, which we can't reach. This translation sits at
//! the boundary between the model's dialect and Codex's tool registry —
//! exactly the layer where translation belongs.

use serde_json::Value as JsonValue;

/// Names the local model may emit as tool names that should actually be
/// executed via the `shell` tool. Comprehensive Linux developer environment
/// coverage; not all entries need to be in the LightCoder whitelist for the
/// alias to fire — these aliases run AFTER the model has emitted a call.
///
/// Excluded by design: interactive editors (`vim`, `nano`, `emacs`, `less`,
/// `more`, `top`, `htop`) — they hang on a non-interactive shell and silently
/// time out. We let those fall through unaliased so the failure is visible.
pub const SHELL_COMMAND_ALIASES: &[&str] = &[
    // --- File system navigation / inspection ---
    "ls",
    "dir",
    "tree",
    "stat",
    "file",
    "find",
    "locate",
    "which",
    "whereis",
    "type",
    "command",
    "basename",
    "dirname",
    "realpath",
    "readlink",
    "pwd",
    "cd",
    // --- File reading ---
    "cat",
    "head",
    "tail",
    "tac",
    "nl",
    "wc",
    "hexdump",
    "xxd",
    "od",
    "strings",
    // --- File / dir manipulation ---
    "touch",
    "mkdir",
    "rmdir",
    "rm",
    "cp",
    "mv",
    "ln",
    "chmod",
    "chown",
    "chgrp",
    "install",
    "truncate",
    // --- Text processing ---
    "echo",
    "printf",
    "sed",
    "awk",
    "tr",
    "cut",
    "paste",
    "sort",
    "uniq",
    "comm",
    "diff",
    "patch",
    "tee",
    "xargs",
    "fmt",
    "fold",
    "expand",
    "unexpand",
    "column",
    "rev",
    "split",
    "csplit",
    // --- Search ---
    "grep",
    "rg",
    "ag",
    "ack",
    "fgrep",
    "egrep",
    // --- Compression / archives ---
    "tar",
    "gzip",
    "gunzip",
    "zcat",
    "gzcat",
    "zip",
    "unzip",
    "bzip2",
    "bunzip2",
    "bzcat",
    "xz",
    "unxz",
    "xzcat",
    "zstd",
    "unzstd",
    // --- Version control ---
    "git",
    "hg",
    "svn",
    // --- Network (read-only / safe) ---
    "curl",
    "wget",
    "ping",
    "host",
    "dig",
    "nslookup",
    "traceroute",
    "mtr",
    // --- Network (potentially destructive — still safe to alias; sandbox enforces) ---
    "ssh",
    "scp",
    "sftp",
    "rsync",
    "nc",
    "netcat",
    "telnet",
    // --- Process / system info ---
    "ps",
    "kill",
    "killall",
    "pgrep",
    "pkill",
    "jobs",
    "free",
    "df",
    "du",
    "mount",
    "umount",
    "lsblk",
    "lscpu",
    "lsof",
    "uptime",
    "who",
    "whoami",
    "id",
    "hostname",
    "uname",
    "date",
    "cal",
    // --- Shell / env ---
    "env",
    "export",
    "set",
    "unset",
    "alias",
    "source",
    "eval",
    "exec",
    "bash",
    "sh",
    "zsh",
    "fish",
    "dash",
    // --- Package managers (system) ---
    "apt",
    "apt-get",
    "yum",
    "dnf",
    "pacman",
    "brew",
    "snap",
    "flatpak",
    "rpm",
    "dpkg",
    // --- Package managers (language) ---
    "pip",
    "pip3",
    "pipx",
    "poetry",
    "conda",
    "mamba",
    "uv",
    "npm",
    "yarn",
    "pnpm",
    "bunx",
    "npx",
    "bundle",
    "bundler",
    "gem",
    "composer",
    "nuget",
    // --- Languages / interpreters ---
    "python",
    "python3",
    "ruby",
    "perl",
    "node",
    "deno",
    "bun",
    "java",
    "javac",
    "kotlin",
    "kotlinc",
    "scala",
    "scalac",
    "sbt",
    "go",
    "gofmt",
    "rustc",
    "gcc",
    "g++",
    "clang",
    "clang++",
    "cc",
    "ld",
    "as",
    "ghc",
    "stack",
    "cabal",
    "ocaml",
    "ocamlfind",
    "dune",
    "php",
    "lua",
    "luac",
    "dart",
    "swift",
    "swiftc",
    "julia",
    "Rscript",
    "R",
    "nim",
    "zig",
    "v",
    "crystal",
    // --- Build systems ---
    "make",
    "cmake",
    "ninja",
    "meson",
    "mvn",
    "gradle",
    "ant",
    "cargo",
    "nix",
    "nix-build",
    "nix-shell",
    "bazel",
    "buck",
    "buck2",
    "pants",
    "just",
    // --- Test runners ---
    "pytest",
    "unittest",
    "tox",
    "jest",
    "vitest",
    "mocha",
    "tap",
    "ava",
    "rspec",
    "minitest",
    "phpunit",
    "pest",
    // --- Cloud / infra / containers ---
    "docker",
    "podman",
    "buildah",
    "skopeo",
    "kubectl",
    "helm",
    "kustomize",
    "k9s",
    "aws",
    "gcloud",
    "az",
    "doctl",
    "linode",
    "terraform",
    "pulumi",
    "ansible",
    "salt",
    "puppet",
    "sam",
    "serverless",
    "vercel",
    "netlify",
    "fly",
    "railway",
    // --- Data / formats ---
    "jq",
    "yq",
    "tomlq",
    "fx",
    "base64",
    "uuencode",
    "uudecode",
    // --- Crypto / hashing ---
    "md5sum",
    "sha1sum",
    "sha256sum",
    "sha512sum",
    "cksum",
    "openssl",
    "gpg",
    "ssh-keygen",
    // --- Misc / coordination ---
    "sleep",
    "watch",
    "time",
    "timeout",
    "yes",
    "true",
    "false",
    "sudo",
    "su",
    "doas",
    "screen",
    "tmux",
    "nohup",
    "disown",
    "history",
    "tput",
    "clear",
    "reset",
    // --- Accessibility / introspection of *this* sandbox ---
    "ulimit",
    "umask",
    "trap",
];

/// Returns true if `name` is a recognized shell-command alias.
pub fn is_shell_command_alias(name: &str) -> bool {
    SHELL_COMMAND_ALIASES.contains(&name)
}

/// Result of translating a tool call.
pub struct TranslatedCall {
    /// The new tool name — always `shell`.
    pub name: &'static str,
    /// New JSON arguments for the `shell` tool: `{ "command": ["bash", "-lc", ...] }`.
    pub args: JsonValue,
    /// The reconstructed shell command line, for logging.
    pub command_line: String,
}

/// Normalize an `apply_patch` invocation. Two normalizations apply, in order:
///
/// 1. **Unified-diff translation** — when the model emits a standard unified
///    diff (the format `git diff` produces, with `--- a/path` / `+++ b/path`
///    headers and `@@ -L,N +L,N @@` hunks), translate it to Codex's native
///    patch format. Models reach for unified diff because that's what their
///    training corpus is full of; rather than fight that prior, accept it.
/// 2. **Prefix repair** — local models often emit hunk bodies WITHOUT the
///    required `+`/`-`/space prefix on each line (they think they're
///    pasting a file body, not a diff). Detect that and add the missing
///    prefix. Also auto-appends `*** End Patch` when missing.
///
/// Returns `Some(translated)` only when at least one normalization fired.
pub fn normalize_apply_patch_call(args: &JsonValue) -> Option<TranslatedCall> {
    let obj = args.as_object()?;
    let input = obj
        .get("input")
        .or_else(|| obj.get("patch"))
        .and_then(|v| v.as_str())?;

    let mut working = input.to_string();
    let mut applied: Vec<&str> = Vec::new();

    if let Some(translated) = translate_unified_diff_to_codex(&working) {
        working = translated;
        applied.push("unified-diff translation");
    }
    if let Some(collapsed) = collapse_repeated_patch_wrappers(&working) {
        working = collapsed;
        applied.push("collapsed repeated wrappers");
    }
    if let Some(fixed_add) = fix_add_file_blocks(&working) {
        working = fixed_add;
        applied.push("stripped @@/- from Add File");
    }
    if let Some(fixed) = fix_apply_patch_body(&working) {
        working = fixed;
        applied.push("fixed prefixes");
    }

    if applied.is_empty() {
        return None;
    }

    let mut new_args = obj.clone();
    new_args.insert(
        "input".to_string(),
        serde_json::Value::String(working.clone()),
    );
    new_args.remove("patch");

    Some(TranslatedCall {
        name: "apply_patch",
        args: serde_json::Value::Object(new_args),
        command_line: format!(
            "apply_patch ({}, {} bytes)",
            applied.join(" + "),
            working.len()
        ),
    })
}

/// If `input` looks like a standard unified diff (`--- a/path` / `+++ b/path`
/// with `@@ -L,N +L,N @@` hunk headers), translate it into Codex's native
/// patch format. Returns `None` for inputs that aren't unified diffs (which
/// includes inputs that are already in Codex format).
///
/// Translations applied:
/// - File pairs `--- a/<path>` + `+++ b/<path>` → `*** Update File: <path>`
/// - File pair `--- /dev/null` + `+++ b/<path>` → `*** Add File: <path>`
/// - File pair `--- a/<path>` + `+++ /dev/null` → `*** Delete File: <path>`
/// - Hunk header `@@ -L,N +L,N @@ <ctx>` → `@@ <ctx>` (Codex matches by
///   context, not line numbers; the optional anchor text is preserved when
///   the model included one)
/// - `@@ -L,N +L,N @@` (no anchor) → `@@`
/// - Body lines (`+`, `-`, ` `) pass through unchanged
/// - Wrapped with `*** Begin Patch` / `*** End Patch`
///
/// The path-prefix conventions `a/` and `b/` come from `git diff`; they're
/// stripped since the working directory is implicit. Bare `<path>` (no `a/`
/// or `b/` prefix, as `diff -u` produces) is also accepted.
pub fn translate_unified_diff_to_codex(input: &str) -> Option<String> {
    let lines: Vec<&str> = input.lines().collect();
    if !looks_like_unified_diff(&lines) {
        return None;
    }

    let mut out = String::with_capacity(input.len() + 64);
    out.push_str("*** Begin Patch\n");

    let mut i = 0;
    let mut produced_any_file = false;
    while i < lines.len() {
        let line = lines[i];

        // Skip git's noise headers ("diff --git ...", "index abc..def", etc.)
        if line.starts_with("diff --git ")
            || line.starts_with("index ")
            || line.starts_with("similarity index")
            || line.starts_with("rename from ")
            || line.starts_with("rename to ")
            || line.starts_with("new file mode")
            || line.starts_with("deleted file mode")
            || line.starts_with("old mode")
            || line.starts_with("new mode")
        {
            i += 1;
            continue;
        }

        // File header pair: --- followed by +++.
        if let Some(old_path_raw) = line.strip_prefix("--- ") {
            let next = lines.get(i + 1)?;
            let new_path_raw = next.strip_prefix("+++ ")?;
            let header = file_header(old_path_raw, new_path_raw)?;
            out.push_str(&header);
            out.push('\n');
            produced_any_file = true;
            i += 2;
            continue;
        }

        // Hunk header: @@ -L,N +L,N @@ [optional anchor]
        if let Some(rest) = line.strip_prefix("@@") {
            let translated = translate_hunk_header(rest);
            out.push_str(&translated);
            out.push('\n');
            i += 1;
            continue;
        }

        // Anything else in a unified-diff context is body content (+, -, or
        // space-prefixed) or a "\ No newline at end of file" marker. Drop
        // the no-newline marker; pass the rest through verbatim.
        if line.starts_with("\\ No newline") {
            i += 1;
            continue;
        }
        out.push_str(line);
        out.push('\n');
        i += 1;
    }

    if !produced_any_file {
        // Looked unified-diff-ish but no file headers — bail out and let the
        // input pass through unchanged.
        return None;
    }

    out.push_str("*** End Patch\n");
    Some(out)
}

/// Returns true iff the input has the structural markers of a unified diff:
/// at least one `--- ` / `+++ ` file-header pair followed by a `@@` hunk
/// header. Inputs that already start with `*** Begin Patch` are explicitly
/// rejected so we don't double-process Codex-format input.
fn looks_like_unified_diff(lines: &[&str]) -> bool {
    if lines.iter().any(|l| l.starts_with("*** Begin Patch")) {
        return false;
    }
    let mut saw_minus_header = false;
    let mut saw_plus_header_after = false;
    let mut saw_hunk_header_after = false;
    for line in lines {
        if !saw_minus_header && line.starts_with("--- ") {
            saw_minus_header = true;
            continue;
        }
        if saw_minus_header && !saw_plus_header_after && line.starts_with("+++ ") {
            saw_plus_header_after = true;
            continue;
        }
        if saw_plus_header_after && line.starts_with("@@") {
            saw_hunk_header_after = true;
            break;
        }
    }
    saw_minus_header && saw_plus_header_after && saw_hunk_header_after
}

/// Build the Codex file header (`*** Add File:`, `*** Update File:`, or
/// `*** Delete File:`) from a unified-diff `--- ` / `+++ ` pair. Returns
/// `None` when the pair is unparseable.
fn file_header(old_path_raw: &str, new_path_raw: &str) -> Option<String> {
    let old_path = strip_diff_path_decoration(old_path_raw);
    let new_path = strip_diff_path_decoration(new_path_raw);
    let old_is_null = old_path == "/dev/null";
    let new_is_null = new_path == "/dev/null";
    match (old_is_null, new_is_null) {
        (true, true) => None,
        (true, false) => Some(format!("*** Add File: {new_path}")),
        (false, true) => Some(format!("*** Delete File: {old_path}")),
        (false, false) => Some(format!("*** Update File: {new_path}")),
    }
}

/// Remove the `a/`/`b/` git prefix and any trailing tab-delimited timestamp
/// metadata that `diff -u` appends.
fn strip_diff_path_decoration(raw: &str) -> String {
    let trimmed = raw.trim();
    // `diff -u` emits "path\tYYYY-MM-DD HH:MM:SS.NNN +TZ"; cut at the tab.
    let no_tab = trimmed.split('\t').next().unwrap_or(trimmed).trim();
    let stripped = no_tab
        .strip_prefix("a/")
        .or_else(|| no_tab.strip_prefix("b/"))
        .unwrap_or(no_tab);
    stripped.to_string()
}

/// Translate the portion of a hunk header that follows the leading `@@`.
/// Examples:
///   ` -17,7 +17,7 @@`                     → `@@`
///   ` -17,7 +17,7 @@ def my_function():`  → `@@ def my_function():`
///   ``                                     → `@@` (already empty)
fn translate_hunk_header(rest: &str) -> String {
    // Strip a leading ` -L[,N] +L[,N] @@` segment if present, then preserve
    // any anchor text the model put after the second `@@`.
    let trimmed = rest.trim_start();
    if let Some(after_minus) = trimmed.strip_prefix('-') {
        // Skip "L[,N] +L[,N] @@" and read the optional trailing anchor.
        if let Some(after_at_at) = find_segment_after_at_at(after_minus) {
            let anchor = after_at_at.trim();
            if anchor.is_empty() {
                return "@@".to_string();
            } else {
                return format!("@@ {anchor}");
            }
        }
    }
    // Couldn't recognize the line-number form; pass through with the leading
    // `@@` re-attached so the existing anchor-line semantics still work.
    if rest.is_empty() || rest == " " {
        "@@".to_string()
    } else {
        format!("@@{rest}")
    }
}

/// Helper for `translate_hunk_header`: given the text after the leading `-`
/// (so it starts with `L,N +L,N @@ ...`), return the substring after the
/// closing `@@`. Returns `None` if no closing `@@` is found.
fn find_segment_after_at_at(s: &str) -> Option<&str> {
    s.find("@@").map(|idx| &s[idx + 2..])
}

/// Normalize the text after `@@` in a Codex-envelope hunk header. Strips a
/// leading ` -L[,N] +L[,N] @@` unified-diff segment if present, preserving
/// any anchor text the model put after the second `@@`. The return value
/// does NOT include the leading `@@` — the caller concatenates it back.
///
/// Examples:
///   ``                                     → ``                (no change)
///   ` def my_function():`                  → ` def my_function():` (no change)
///   ` -1,6 +1,6 @@`                        → ``                (stripped)
///   ` -17,7 +17,7 @@ def my_function():`   → ` def my_function():`
fn normalize_codex_hunk_header(rest: &str) -> String {
    let trimmed = rest.trim_start();
    let Some(after_minus) = trimmed.strip_prefix('-') else {
        return rest.to_string();
    };
    // Expect digits immediately after the `-`; otherwise it's a real
    // anchor line that happens to start with `-` (unusual but possible).
    if !after_minus.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return rest.to_string();
    }
    // Require a closing `@@` somewhere after, otherwise this isn't a
    // line-number header — could be raw content starting with `-`.
    let Some(at_at_idx) = after_minus.find("@@") else {
        return rest.to_string();
    };
    let after_at_at = &after_minus[at_at_idx + 2..];
    let anchor = after_at_at.trim();
    if anchor.is_empty() {
        String::new()
    } else {
        format!(" {anchor}")
    }
}

/// Pre-pass: collapse multiple `*** Begin Patch ... *** End Patch` wrappers
/// into a single one. Some local models emit one wrapper per file when
/// patching multiple files; the apply_patch parser only accepts a single
/// wrapper containing multiple Add/Update/Delete operations. Returns
/// `Some(rewritten)` if a collapse was needed, otherwise `None`.
fn collapse_repeated_patch_wrappers(input: &str) -> Option<String> {
    let begin_count = input.matches("*** Begin Patch").count();
    if begin_count <= 1 {
        return None;
    }
    // Walk lines: keep the first `*** Begin Patch`, drop every subsequent
    // `*** Begin Patch` and every non-final `*** End Patch`, keep the last
    // `*** End Patch` (or none if missing — the prefix-fixer will add one).
    let end_count = input.matches("*** End Patch").count();
    let mut seen_begin = 0usize;
    let mut seen_end = 0usize;
    let mut out = String::with_capacity(input.len());
    for raw_line in input.split_inclusive('\n') {
        let trimmed = raw_line.trim_end_matches(['\n', '\r']);
        if trimmed == "*** Begin Patch" {
            seen_begin += 1;
            if seen_begin > 1 {
                continue; // drop duplicate wrapper opener
            }
        } else if trimmed == "*** End Patch" {
            seen_end += 1;
            if seen_end < end_count {
                continue; // drop intermediate wrapper closer
            }
        }
        out.push_str(raw_line);
    }
    Some(out)
}

/// Pre-pass: when an `*** Add File: <path>` block is followed by `@@` hunk
/// headers and `-` lines (Update File–style), strip them and keep only
/// the `+` lines as the new file's content. Add File creates a new file
/// and only accepts `+` lines — `@@` and `-` are rejected by the parser.
/// Returns `Some(rewritten)` if a fix was applied, else `None`.
fn fix_add_file_blocks(input: &str) -> Option<String> {
    let mut changed = false;
    let mut out = String::with_capacity(input.len());
    let mut in_add_file = false;
    for raw_line in input.split_inclusive('\n') {
        let trimmed = raw_line.trim_end_matches(['\n', '\r']);
        if trimmed.starts_with("*** Add File:") {
            in_add_file = true;
            out.push_str(raw_line);
            continue;
        }
        if trimmed.starts_with("*** End Patch")
            || trimmed.starts_with("*** Update File:")
            || trimmed.starts_with("*** Delete File:")
            || trimmed.starts_with("*** End of File")
        {
            in_add_file = false;
            out.push_str(raw_line);
            continue;
        }
        if in_add_file {
            // In Add File: drop `@@` headers and `-` lines outright.
            // Keep `+` lines (real content), and keep blank lines.
            if trimmed.starts_with("@@") {
                changed = true;
                continue;
            }
            if trimmed.starts_with('-') {
                changed = true;
                continue;
            }
        }
        out.push_str(raw_line);
    }
    if changed { Some(out) } else { None }
}

/// Walk an apply_patch body and prefix any bare content lines with `+`,
/// and auto-append the `*** End Patch` terminator when missing. Returns
/// `None` if no fix was needed.
fn fix_apply_patch_body(input: &str) -> Option<String> {
    let mut output = String::with_capacity(input.len());
    let mut in_hunk = false;
    let mut changed = false;

    for raw_line in input.split_inclusive('\n') {
        // Strip the trailing newline if present so we can match cleanly,
        // remembering whether to re-add it.
        let (line, newline) = match raw_line.strip_suffix('\n') {
            Some(stripped) => (stripped, "\n"),
            None => (raw_line, ""),
        };

        // Patch envelope markers — never modify, but they reset hunk state.
        if line.starts_with("*** Begin Patch")
            || line.starts_with("*** End Patch")
            || line.starts_with("*** Add File:")
            || line.starts_with("*** Update File:")
            || line.starts_with("*** Delete File:")
            || line.starts_with("*** End of File")
        {
            in_hunk = line.starts_with("*** Add File:") || line.starts_with("*** Update File:");
            output.push_str(line);
            output.push_str(newline);
            continue;
        }

        // Hunk context markers (`@@ ... @@`) start a hunk window but are
        // themselves headers, not content. If the model emitted a
        // unified-diff-style header like `@@ -1,6 +1,6 @@` (or
        // `@@ -17,7 +17,7 @@ def foo():`) inside an otherwise-Codex patch,
        // strip the line-number segment — Codex apply_patch treats whatever
        // follows `@@ ` as a literal anchor line, and the line-number form
        // will always fail to match. This is the hybrid case the full
        // unified-diff translator skips because the envelope itself is
        // already Codex format.
        if line.starts_with("@@") {
            in_hunk = true;
            let rest = &line[2..];
            let normalized_header = normalize_codex_hunk_header(rest);
            let new_header_line = if normalized_header == rest {
                line.to_string()
            } else {
                changed = true;
                format!("@@{normalized_header}")
            };
            output.push_str(&new_header_line);
            output.push_str(newline);
            continue;
        }

        if !in_hunk {
            output.push_str(line);
            output.push_str(newline);
            continue;
        }

        // Inside a hunk: lines starting with +, -, or a leading space are
        // already correctly prefixed. Empty lines are also fine (they
        // represent context blank lines and Codex's parser accepts them).
        // Any other line is bare content that the model forgot to prefix.
        let already_prefixed = line.starts_with('+')
            || line.starts_with('-')
            || line.starts_with(' ')
            || line.is_empty();
        if already_prefixed {
            output.push_str(line);
            output.push_str(newline);
            continue;
        }

        output.push('+');
        output.push_str(line);
        output.push_str(newline);
        changed = true;
    }

    // Auto-append `*** End Patch` if the body has at least one `*** Begin Patch`
    // but no closing terminator. Models commonly forget the closing marker.
    let trimmed_end = output.trim_end_matches(['\n', '\r']);
    let has_begin = output.contains("*** Begin Patch");
    let has_end = trimmed_end.ends_with("*** End Patch")
        || output.contains("\n*** End Patch\n")
        || output.contains("\n*** End Patch");
    if has_begin && !has_end {
        if !output.ends_with('\n') {
            output.push('\n');
        }
        output.push_str("*** End Patch\n");
        changed = true;
    }

    if changed { Some(output) } else { None }
}

/// Translate a model-emitted tool call to a properly-shaped `shell` call.
///
/// Returns `Some(translated)` in two cases:
///
/// 1. The model called a recognized shell-command alias (e.g. `ls`, `git`):
///    builds a full command line from the alias name + heuristically-extracted
///    args, wrapped as `shell({"command": ["bash", "-lc", "<cmd>"]})`.
///
/// 2. The model called `shell` itself but passed `command` as a string instead
///    of the required array (e.g. `shell({"command": "ls -la"})`): wraps it as
///    `shell({"command": ["bash", "-lc", "ls -la"]})` so Codex's strict tool
///    schema accepts it.
///
/// Returns `None` for anything else — the call passes through unchanged.
///
/// Argument extraction is heuristic — different models pack the args
/// differently. We try the common conventions in order:
///   - `command`/`cmd`/`args`/`argv`/`input` → string or array of args
///   - `path`/`file`/`filename`/`target`/`dir`/`url`/`query`/`pattern` → single positional
///   - everything else → flag-style `--key=value`
pub fn translate_to_shell_call(name: &str, args: &JsonValue) -> Option<TranslatedCall> {
    if name == "shell" {
        return normalize_shell_args(args);
    }
    if !is_shell_command_alias(name) {
        return None;
    }
    let arg_str = extract_args_string(args);
    let command_line = if arg_str.is_empty() {
        name.to_string()
    } else {
        format!("{name} {arg_str}")
    };
    Some(TranslatedCall {
        name: "shell",
        args: serde_json::json!({
            "command": ["bash", "-lc", command_line.clone()],
        }),
        command_line,
    })
}

/// Normalize a `shell` call's `command` field. The schema expects an array of
/// strings (typically `["bash", "-lc", "<command>"]`); local models commonly
/// produce two malformed shapes:
///
/// - String instead of array: `{"command": "ls -la"}` — wrap as
///   `["bash", "-lc", "ls -la"]`.
/// - Double-wrapped array: `["bash", "-lc", "[\"bash\",\"-lc\",\"<cmd>\"]"]`
///   where the third element is the literal JSON of an inner bash invocation
///   — unwrap the inner command line.
///
/// Returns `None` when the call already conforms to the schema.
fn normalize_shell_args(args: &JsonValue) -> Option<TranslatedCall> {
    let obj = args.as_object()?;
    let command = obj.get("command")?;

    // Array case: check for both already-correct and double-wrapped shapes.
    if let Some(arr) = command.as_array() {
        if !arr.iter().all(|v| v.is_string()) {
            return None;
        }
        // Detect double-wrap: ["bash", "-lc", "[\"bash\",\"-lc\",\"<cmd>\"]"]
        if let Some(inner_cmd) = detect_double_wrap(arr) {
            let mut new_args = obj.clone();
            new_args.insert(
                "command".to_string(),
                serde_json::json!(["bash", "-lc", inner_cmd.clone()]),
            );
            return Some(TranslatedCall {
                name: "shell",
                args: serde_json::Value::Object(new_args),
                command_line: inner_cmd,
            });
        }
        // Otherwise the call already conforms.
        return None;
    }

    // String form: wrap with bash -lc — but first unwrap if the string is
    // itself a JSON-encoded shell array, the most common malformed shape we
    // see from local models (`command: "[\"bash\", \"-lc\", \"ls\"]"`).
    let command_str = command
        .as_str()
        .or_else(|| obj.get("cmd").and_then(|v| v.as_str()))?;
    let command_line =
        unwrap_json_shell_string(command_str).unwrap_or_else(|| command_str.trim().to_string());
    if command_line.is_empty() {
        return None;
    }

    // Preserve any other fields the caller passed (e.g. `workdir`, `timeout_ms`).
    let mut new_args = obj.clone();
    new_args.insert(
        "command".to_string(),
        serde_json::json!(["bash", "-lc", command_line.clone()]),
    );
    new_args.remove("cmd");

    Some(TranslatedCall {
        name: "shell",
        args: serde_json::Value::Object(new_args),
        command_line,
    })
}

/// If `s` is a JSON array of strings whose first two elements look like a
/// shell + flag (e.g. `["bash", "-lc", "<cmd>"]`), return the joined inner
/// command line. Otherwise return `None` and let the caller treat `s` as a
/// raw shell line.
fn unwrap_json_shell_string(s: &str) -> Option<String> {
    let trimmed = s.trim();
    if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
        return None;
    }
    // Same two-pass approach as detect_double_wrap: try strict parse first,
    // then re-escape control chars inside strings before retrying. Models
    // commonly include literal newlines (heredoc bodies) which break the
    // strict JSON parse.
    if let Some(parsed) = parse_shell_array(trimmed) {
        return Some(parsed);
    }
    let escaped = escape_control_chars_in_strings(trimmed);
    parse_shell_array(&escaped)
}

/// Detect the model's "double-wrapped bash" mistake. Returns the inner command
/// line if the array looks like `["bash", "-lc", "[\"bash\",\"-lc\",\"<cmd>\"]"]`
/// or a close variant.
fn detect_double_wrap(arr: &[JsonValue]) -> Option<String> {
    // Need at least bash + -lc + payload.
    let last = arr.last()?.as_str()?.trim();
    if !last.starts_with('[') || !last.ends_with(']') {
        return None;
    }

    // Strict JSON parse first — covers the clean case.
    if let Some(parsed) = parse_shell_array(last) {
        return Some(parsed);
    }

    // Strict JSON failed. The most common reason is unescaped control
    // characters inside the inner heredoc body (literal `\n`, `\t`, `\r`).
    // Re-escape control chars inside string literals before re-parsing —
    // serde_json will unescape them back to literal characters in the
    // resulting `String` values.
    let escaped = escape_control_chars_in_strings(last);
    parse_shell_array(&escaped)
}

/// Parse a string of the form `["shell", "flag", "cmd"...]` and return the
/// joined `cmd...` if it matches the shell-prefix shape. Returns `None` on any
/// parse failure or if the array doesn't look like a shell invocation.
fn parse_shell_array(s: &str) -> Option<String> {
    let inner: Vec<JsonValue> = serde_json::from_str(s).ok()?;
    if inner.len() < 3 {
        return None;
    }
    let inner_strs: Vec<&str> = inner.iter().filter_map(|v| v.as_str()).collect();
    if inner_strs.len() != inner.len() {
        return None;
    }
    let shell_like = matches!(
        inner_strs[0],
        "bash" | "sh" | "zsh" | "/bin/bash" | "/bin/sh"
    );
    let flag_like = matches!(inner_strs[1], "-c" | "-lc" | "-l");
    if !shell_like || !flag_like {
        return None;
    }
    Some(inner_strs[2..].join(" "))
}

/// Walk the input and escape `\n`, `\r`, `\t` inside JSON string literals.
/// Tracks whether we're currently inside a `"..."` to avoid touching
/// structural whitespace. Backslash-escaped quotes are honored.
fn escape_control_chars_in_strings(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    let mut in_string = false;
    let mut prev = '\0';
    for c in s.chars() {
        if c == '"' && prev != '\\' {
            in_string = !in_string;
            out.push(c);
        } else if in_string {
            match c {
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                other => out.push(other),
            }
        } else {
            out.push(c);
        }
        prev = c;
    }
    out
}

fn extract_args_string(args: &JsonValue) -> String {
    if args.is_null() {
        return String::new();
    }

    // Some models emit arguments as a raw string rather than an object.
    if let Some(s) = args.as_str() {
        return s.trim().to_string();
    }

    let Some(obj) = args.as_object() else {
        return String::new();
    };
    if obj.is_empty() {
        return String::new();
    }

    // 1. Common "rest of command" fields.
    for key in &[
        "command",
        "cmd",
        "args",
        "argv",
        "input",
        "expression",
        "code",
    ] {
        if let Some(v) = obj.get(*key) {
            if let Some(s) = v.as_str() {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    return trimmed.to_string();
                }
            }
            if let Some(arr) = v.as_array() {
                let joined = arr
                    .iter()
                    .filter_map(|a| a.as_str().map(shell_quote_if_needed))
                    .collect::<Vec<_>>()
                    .join(" ");
                if !joined.is_empty() {
                    return joined;
                }
            }
        }
    }

    // 2. Single-positional path-shaped fields.
    for key in &[
        "path",
        "file",
        "filename",
        "filepath",
        "target",
        "dir",
        "directory",
        "folder",
        "url",
        "uri",
        "query",
        "pattern",
        "search",
    ] {
        if let Some(v) = obj.get(*key).and_then(|v| v.as_str()) {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                return shell_quote_if_needed(trimmed);
            }
        }
    }

    // 3. Last resort — flag-style serialization. This catches things like
    //    `chmod({"mode": "755", "file": "x"})` → `--mode=755 --file=x`. Not
    //    perfect but better than dropping args entirely.
    obj.iter()
        .filter_map(|(k, v)| {
            v.as_str()
                .or_else(|| v.as_bool().map(|b| if b { "true" } else { "false" }))
                .map(|s| format!("--{k}={}", shell_quote_if_needed(s)))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Quote a single argument if it contains shell metacharacters.
fn shell_quote_if_needed(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    let needs_quote = s.chars().any(|c| {
        matches!(
            c,
            ' ' | '\t'
                | '\n'
                | '"'
                | '\''
                | '$'
                | '`'
                | '\\'
                | '|'
                | '&'
                | ';'
                | '<'
                | '>'
                | '('
                | ')'
                | '#'
                | '*'
                | '?'
                | '['
                | ']'
                | '!'
                | '{'
                | '}'
        )
    });
    if !needs_quote {
        return s.to_string();
    }
    // Single-quote and escape any embedded single quotes.
    let escaped = s.replace('\'', r"'\''");
    format!("'{escaped}'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_common_unix_commands() {
        for name in ["ls", "cat", "grep", "rg", "find", "git", "make", "cargo"] {
            assert!(is_shell_command_alias(name), "missing alias for {name}");
        }
    }

    #[test]
    fn ignores_known_codex_tools() {
        // None of these should be in the shell-command alias set.
        // (`shell` itself has a separate normalization path.)
        for name in [
            "shell",
            "apply_patch",
            "list_dir",
            "view_image",
            "local_web_search",
        ] {
            assert!(
                !is_shell_command_alias(name),
                "should not alias {name} (it's a real Codex tool)"
            );
        }
    }

    #[test]
    fn shell_with_string_command_gets_normalized_to_array() {
        let result =
            translate_to_shell_call("shell", &serde_json::json!({"command": "ls -la"})).unwrap();
        assert_eq!(result.name, "shell");
        assert_eq!(result.command_line, "ls -la");
        assert_eq!(
            result.args,
            serde_json::json!({"command": ["bash", "-lc", "ls -la"]})
        );
    }

    #[test]
    fn shell_with_correct_array_command_passes_through() {
        let result = translate_to_shell_call(
            "shell",
            &serde_json::json!({"command": ["bash", "-lc", "ls"]}),
        );
        assert!(
            result.is_none(),
            "correct shell shape should not be re-translated"
        );
    }

    #[test]
    fn shell_double_wrap_is_unwrapped() {
        let result = translate_to_shell_call(
            "shell",
            &serde_json::json!({
                "command": ["bash", "-lc", "[\"bash\",\"-lc\",\"cat foo.txt\"]"]
            }),
        )
        .unwrap();
        assert_eq!(result.command_line, "cat foo.txt");
        assert_eq!(
            result.args,
            serde_json::json!({"command": ["bash", "-lc", "cat foo.txt"]})
        );
    }

    #[test]
    fn shell_double_wrap_with_multiline_heredoc_is_unwrapped() {
        // Real failure pattern: model double-wraps a heredoc that contains
        // embedded newlines. The naive `serde_json::from_str` fails because
        // JSON requires control characters in strings to be escaped — the
        // raw `\n` in the inner string is invalid JSON.
        let inner = "cat > handle-resolver.ts << 'EOF'\nimport { fetch } from 'undici'\n\ninterface X {}\nEOF";
        let wrapped_third = format!("[\"bash\",\"-lc\",\"{inner}\"]");
        let result = translate_to_shell_call(
            "shell",
            &serde_json::json!({"command": ["bash", "-lc", wrapped_third]}),
        );
        assert!(
            result.is_some(),
            "double-wrap with heredoc/newlines should still be detected"
        );
        let result = result.unwrap();
        assert_eq!(result.command_line, inner);
    }

    /// Mirrors the exact byte shape we observed in the wild from qwen3.5:9b —
    /// a path with multiple slashes, no extra whitespace, the kind of payload
    /// that revealed the original detection bug.
    #[test]
    fn shell_double_wrap_observed_in_wild_is_unwrapped() {
        let inner = "wc -l /home/jesse/src/codex.test.site/tests/test_lambda.py";
        let wrapped_third = format!("[\"bash\",\"-lc\",\"{inner}\"]");
        let result = translate_to_shell_call(
            "shell",
            &serde_json::json!({"command": ["bash", "-lc", wrapped_third]}),
        )
        .expect("double-wrap should be detected");
        assert_eq!(result.command_line, inner);
    }

    #[test]
    fn shell_legitimate_array_brackets_in_command_pass_through() {
        // A command line that legitimately contains array-like syntax (e.g.
        // `python -c '[1,2,3]'`) should NOT be unwrapped — the inner is not a
        // shell-prefixed JSON array.
        let result = translate_to_shell_call(
            "shell",
            &serde_json::json!({
                "command": ["bash", "-lc", "python -c 'print([1,2,3])'"]
            }),
        );
        assert!(result.is_none(), "real shell command should pass through");
    }

    #[test]
    fn shell_with_string_holding_json_shell_array_is_unwrapped() {
        // The model passes `command` as a string, but the string is itself a
        // JSON-encoded shell array — the actual failure mode observed with
        // qwen3.5:9b in local-only mode.
        let result = translate_to_shell_call(
            "shell",
            &serde_json::json!({
                "command": "[\"bash\",\"-lc\",\"wc -l tests/test_lambda.py\"]"
            }),
        )
        .unwrap();
        assert_eq!(result.command_line, "wc -l tests/test_lambda.py");
        assert_eq!(
            result.args,
            serde_json::json!({"command": ["bash", "-lc", "wc -l tests/test_lambda.py"]})
        );
    }

    #[test]
    fn shell_with_string_that_looks_arrayish_but_isnt_passes_through() {
        // A string command that LOOKS like it has brackets but isn't a JSON
        // shell array (e.g. `python -c '[1,2,3]'`) should be wrapped normally.
        let result = translate_to_shell_call(
            "shell",
            &serde_json::json!({"command": "python -c 'print([1,2,3])'"}),
        )
        .unwrap();
        assert_eq!(result.command_line, "python -c 'print([1,2,3])'");
    }

    #[test]
    fn apply_patch_with_missing_plus_prefix_gets_fixed() {
        // The model commonly drops the `+` prefix on add-file content lines.
        let input =
            "*** Begin Patch\n*** Add File: hello.py\nimport sys\nprint('hi')\n*** End Patch\n";
        let result = normalize_apply_patch_call(&serde_json::json!({"input": input})).unwrap();
        let fixed = result.args.get("input").unwrap().as_str().unwrap();
        assert!(
            fixed.contains("+import sys\n"),
            "missing + on import line: {fixed}"
        );
        assert!(
            fixed.contains("+print('hi')\n"),
            "missing + on print line: {fixed}"
        );
        assert!(fixed.contains("*** Begin Patch"));
        assert!(fixed.contains("*** Add File: hello.py"));
    }

    #[test]
    fn apply_patch_missing_end_marker_gets_appended() {
        // Real failure observed: model emits a complete patch body but forgets
        // the trailing `*** End Patch`. Auto-append.
        let input = "*** Begin Patch\n*** Add File: test.ts\n+const a = 1\n+export {}";
        let result = normalize_apply_patch_call(&serde_json::json!({"input": input})).unwrap();
        let fixed = result.args.get("input").unwrap().as_str().unwrap();
        assert!(
            fixed.trim_end().ends_with("*** End Patch"),
            "should auto-append *** End Patch:\n{fixed}"
        );
    }

    #[test]
    fn apply_patch_already_correct_passes_through() {
        let input =
            "*** Begin Patch\n*** Add File: hello.py\n+import sys\n+print('hi')\n*** End Patch\n";
        let result = normalize_apply_patch_call(&serde_json::json!({"input": input}));
        assert!(
            result.is_none(),
            "well-formed patch should not be rewritten"
        );
    }

    #[test]
    fn apply_patch_update_preserves_context_and_minus_lines() {
        let input = "*** Begin Patch\n*** Update File: foo.py\n@@\n-old\n new content line\n+new\n*** End Patch\n";
        // Only `new content line` (without the leading space) would need fixing,
        // but it already has a leading space → context line. Nothing should change.
        let result = normalize_apply_patch_call(&serde_json::json!({"input": input}));
        assert!(result.is_none());
    }

    #[test]
    fn unified_diff_translation_basic_update() {
        let input = "\
--- a/handler.py
+++ b/handler.py
@@ -17,7 +17,7 @@
             \"body\": json.dumps({\"error\": \"Missing 'handle' in event\"})
         }

-    url = f\"https://api.handle.me/resolve/{handle}\"
+    url = f\"https://api.handle.me/handles/{handle}\"

     try:
         response = requests.get(url)
";
        let translated = translate_unified_diff_to_codex(input).unwrap();
        assert!(translated.starts_with("*** Begin Patch\n"));
        assert!(translated.contains("*** Update File: handler.py\n"));
        assert!(translated.contains("@@\n")); // hunk header collapsed (no anchor)
        assert!(translated.contains("-    url = f\"https://api.handle.me/resolve/{handle}\""));
        assert!(translated.contains("+    url = f\"https://api.handle.me/handles/{handle}\""));
        assert!(translated.trim_end().ends_with("*** End Patch"));
    }

    #[test]
    fn unified_diff_translation_preserves_anchor_after_hunk_header() {
        let input = "\
--- a/lib.py
+++ b/lib.py
@@ -1,3 +1,3 @@ def my_function():
-    foo()
+    bar()
     return None
";
        let translated = translate_unified_diff_to_codex(input).unwrap();
        assert!(translated.contains("@@ def my_function():\n"));
    }

    #[test]
    fn unified_diff_translation_dev_null_means_add_file() {
        let input = "\
--- /dev/null
+++ b/new_file.py
@@ -0,0 +1,2 @@
+import sys
+print('hi')
";
        let translated = translate_unified_diff_to_codex(input).unwrap();
        assert!(translated.contains("*** Add File: new_file.py\n"));
        assert!(!translated.contains("*** Update File:"));
    }

    #[test]
    fn unified_diff_translation_dev_null_means_delete_file() {
        let input = "\
--- a/old_file.py
+++ /dev/null
@@ -1,2 +0,0 @@
-import sys
-print('hi')
";
        let translated = translate_unified_diff_to_codex(input).unwrap();
        assert!(translated.contains("*** Delete File: old_file.py\n"));
    }

    #[test]
    fn unified_diff_translation_skips_git_noise_headers() {
        let input = "\
diff --git a/foo.py b/foo.py
index abc1234..def5678 100644
--- a/foo.py
+++ b/foo.py
@@ -1 +1 @@
-old
+new
";
        let translated = translate_unified_diff_to_codex(input).unwrap();
        assert!(!translated.contains("diff --git"));
        assert!(!translated.contains("index abc"));
        assert!(translated.contains("*** Update File: foo.py\n"));
    }

    #[test]
    fn unified_diff_translation_returns_none_for_codex_format() {
        let input = "*** Begin Patch\n*** Add File: hello.py\n+import sys\n*** End Patch\n";
        assert!(translate_unified_diff_to_codex(input).is_none());
    }

    #[test]
    fn unified_diff_translation_returns_none_for_unrelated_text() {
        let input = "Hello world, this is not a diff at all.";
        assert!(translate_unified_diff_to_codex(input).is_none());
    }

    #[test]
    fn unified_diff_strips_no_newline_marker() {
        let input = "\
--- a/foo.py
+++ b/foo.py
@@ -1 +1 @@
-old
+new
\\ No newline at end of file
";
        let translated = translate_unified_diff_to_codex(input).unwrap();
        assert!(!translated.contains("\\ No newline"));
    }

    #[test]
    fn normalize_pipeline_handles_unified_diff_end_to_end() {
        let input = "\
--- a/foo.py
+++ b/foo.py
@@ -1 +1 @@
-old
+new
";
        let result = normalize_apply_patch_call(&serde_json::json!({"input": input})).unwrap();
        let body = result.args.get("input").unwrap().as_str().unwrap();
        assert!(body.starts_with("*** Begin Patch\n"));
        assert!(body.contains("*** Update File: foo.py\n"));
        assert!(body.trim_end().ends_with("*** End Patch"));
        assert!(result.command_line.contains("unified-diff translation"));
    }

    #[test]
    fn codex_hunk_header_with_line_numbers_gets_stripped() {
        let input = "\
*** Begin Patch
*** Update File: handler.py
@@ -1,6 +1,6 @@
 import requests
 import os

-API_BASE_URL = \"https://api.handle.me/resolve/\"
+API_BASE_URL = \"https://api.handle.me/handles/\"

*** End Patch
";
        let result = normalize_apply_patch_call(&serde_json::json!({"input": input})).unwrap();
        let body = result.args.get("input").unwrap().as_str().unwrap();
        // The hybrid hunk header should have been collapsed to bare `@@`.
        assert!(body.contains("\n@@\n"), "expected bare `@@`, got:\n{body}");
        assert!(!body.contains("@@ -1,6 +1,6 @@"));
        // Content around the change is preserved verbatim.
        assert!(body.contains("-API_BASE_URL = \"https://api.handle.me/resolve/\""));
        assert!(body.contains("+API_BASE_URL = \"https://api.handle.me/handles/\""));
    }

    #[test]
    fn codex_hunk_header_with_line_numbers_and_anchor_preserves_anchor() {
        let input = "\
*** Begin Patch
*** Update File: lib.py
@@ -17,7 +17,7 @@ def my_function():
-    foo()
+    bar()
*** End Patch
";
        let result = normalize_apply_patch_call(&serde_json::json!({"input": input})).unwrap();
        let body = result.args.get("input").unwrap().as_str().unwrap();
        // Anchor text is preserved; line numbers are gone.
        assert!(body.contains("@@ def my_function():"), "expected `@@ def my_function():`, got:\n{body}");
        assert!(!body.contains("-17,7"));
    }

    #[test]
    fn codex_hunk_header_with_real_anchor_is_untouched() {
        // A legitimate `@@ <anchor>` form (no line numbers) must pass through
        // unchanged.
        let input = "\
*** Begin Patch
*** Update File: foo.py
@@ def bar():
-    old
+    new
*** End Patch
";
        // Since this patch is already well-formed, normalize should return None.
        let result = normalize_apply_patch_call(&serde_json::json!({"input": input}));
        assert!(result.is_none(), "well-formed @@ anchor should not be rewritten");
    }

    #[test]
    fn codex_hunk_header_bare_at_at_is_untouched() {
        let input = "\
*** Begin Patch
*** Update File: foo.py
@@
-    old
+    new
*** End Patch
";
        let result = normalize_apply_patch_call(&serde_json::json!({"input": input}));
        assert!(result.is_none(), "bare `@@` should not be rewritten");
    }

    #[test]
    fn unified_diff_with_bare_path_no_a_b_prefix() {
        // diff -u (without git) emits paths without the a/ b/ prefix.
        let input = "\
--- foo.py\t2026-04-22 02:00:00.000 +0000
+++ foo.py\t2026-04-22 02:01:00.000 +0000
@@ -1 +1 @@
-old
+new
";
        let translated = translate_unified_diff_to_codex(input).unwrap();
        assert!(translated.contains("*** Update File: foo.py\n"));
    }

    #[test]
    fn shell_normalization_preserves_extra_fields() {
        let result = translate_to_shell_call(
            "shell",
            &serde_json::json!({"command": "cargo test", "workdir": "/tmp/foo", "timeout_ms": 60000}),
        )
        .unwrap();
        assert_eq!(result.args.get("workdir").unwrap(), "/tmp/foo");
        assert_eq!(result.args.get("timeout_ms").unwrap(), 60000);
    }

    #[test]
    fn empty_args_runs_bare_command() {
        let t = translate_to_shell_call("ls", &serde_json::json!({})).unwrap();
        assert_eq!(t.command_line, "ls");
        assert_eq!(
            t.args,
            serde_json::json!({"command": ["bash", "-lc", "ls"]})
        );
    }

    #[test]
    fn command_field_string_is_used_as_args() {
        let t = translate_to_shell_call("ls", &serde_json::json!({"command": "-la"})).unwrap();
        assert_eq!(t.command_line, "ls -la");
    }

    #[test]
    fn args_array_is_joined() {
        let t = translate_to_shell_call(
            "git",
            &serde_json::json!({"argv": ["status", "--porcelain"]}),
        )
        .unwrap();
        assert_eq!(t.command_line, "git status --porcelain");
    }

    #[test]
    fn path_field_used_for_cat() {
        let t = translate_to_shell_call("cat", &serde_json::json!({"path": "src/foo.py"})).unwrap();
        assert_eq!(t.command_line, "cat src/foo.py");
    }

    #[test]
    fn pattern_field_used_for_grep() {
        let t = translate_to_shell_call("grep", &serde_json::json!({"pattern": "TODO"})).unwrap();
        assert_eq!(t.command_line, "grep TODO");
    }

    #[test]
    fn paths_with_spaces_are_quoted() {
        let t =
            translate_to_shell_call("cat", &serde_json::json!({"path": "my file.txt"})).unwrap();
        assert_eq!(t.command_line, "cat 'my file.txt'");
    }

    #[test]
    fn flag_style_fallback() {
        let t = translate_to_shell_call(
            "chmod",
            &serde_json::json!({"mode": "755", "file": "run.sh"}),
        )
        .unwrap();
        // file= field is a path, takes priority over mode= flag — exact result
        // depends on iteration order, but the path-shaped field wins.
        assert_eq!(t.command_line, "chmod run.sh");
    }

    #[test]
    fn unknown_tool_returns_none() {
        let result = translate_to_shell_call("apply_patch", &serde_json::json!({}));
        assert!(result.is_none());
    }

    #[test]
    fn args_string_form_is_accepted() {
        let t = translate_to_shell_call("ls", &serde_json::json!("-la /tmp")).unwrap();
        assert_eq!(t.command_line, "ls -la /tmp");
    }
}

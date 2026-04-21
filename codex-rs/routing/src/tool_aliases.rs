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

/// Normalize an `apply_patch` invocation. Local models often emit the file
/// content WITHOUT the required `+` prefix on each line (they think they're
/// pasting a file body, not a diff). Detect that and add the missing prefix.
///
/// Returns `Some(translated)` only when a fix was actually needed — the
/// resulting `name` is still `apply_patch`, just with corrected `input`.
pub fn normalize_apply_patch_call(args: &JsonValue) -> Option<TranslatedCall> {
    let obj = args.as_object()?;
    let input = obj
        .get("input")
        .or_else(|| obj.get("patch"))
        .and_then(|v| v.as_str())?;

    let fixed = fix_apply_patch_body(input)?;

    let mut new_args = obj.clone();
    new_args.insert(
        "input".to_string(),
        serde_json::Value::String(fixed.clone()),
    );
    new_args.remove("patch");

    Some(TranslatedCall {
        name: "apply_patch",
        args: serde_json::Value::Object(new_args),
        command_line: format!("apply_patch (fixed prefixes, {} bytes)", fixed.len()),
    })
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
        // themselves headers, not content.
        if line.starts_with("@@") {
            in_hunk = true;
            output.push_str(line);
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

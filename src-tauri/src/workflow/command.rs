//! Shell command renderer + spawner. Shared by `entry/*` (which uses the
//! `*_source.command` templates from `EntryConfig` to fetch repos / issues)
//! and the Phase-2 spawn bridge (which uses it to run `claude -p` etc.).
//!
//! The render pass is deliberately minimal: literal `{var}` substitution
//! against a string→string map. Jinja2 lives in `expr.rs` and serves
//! per-issue templates; entry-level commands are simple enough that a
//! tiny replacer keeps `command.rs` decoupled from the expression engine.

use std::collections::HashMap;

#[derive(thiserror::Error, Debug)]
pub enum CommandError {
    #[error("missing template variable: {0}")]
    MissingVar(String),
    #[error("spawn failed: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("command exited non-zero (code={code:?}): {stderr}")]
    NonZeroExit { code: Option<i32>, stderr: String },
    #[error("invalid JSON output from command: {0}")]
    Json(#[from] serde_json::Error),
}

/// Substitute `{var}` placeholders in `template` with values from `vars`.
/// Unknown placeholders return `MissingVar`. `{{` collapses to a literal `{`
/// and `}}` to `}` — needed when a command emits JSON or shell parameter
/// expansions that include real braces.
///
/// Whitespace inside the placeholder is allowed (`{ repo }` matches
/// `{repo}`) so multi-line YAML scalars survive a `serde_yaml` reflow.
pub fn render_template(template: &str, vars: &HashMap<&str, &str>) -> Result<String, CommandError> {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let ch = bytes[i];
        // `{{` → literal `{`
        if ch == b'{' && bytes.get(i + 1) == Some(&b'{') {
            out.push('{');
            i += 2;
            continue;
        }
        // `}}` → literal `}`
        if ch == b'}' && bytes.get(i + 1) == Some(&b'}') {
            out.push('}');
            i += 2;
            continue;
        }
        if ch != b'{' {
            // Walk to the next char boundary safely (multi-byte UTF-8).
            let next = next_char_boundary(template, i);
            out.push_str(&template[i..next]);
            i = next;
            continue;
        }
        // Find the matching '}'. If none, treat the '{' as literal.
        let rest = &template[i + 1..];
        let Some(end_rel) = rest.find('}') else {
            out.push('{');
            i += 1;
            continue;
        };
        let key = rest[..end_rel].trim();
        // Skip empty `{}` — pass through literally.
        if key.is_empty() {
            out.push('{');
            i += 1;
            continue;
        }
        let value = vars
            .get(key)
            .ok_or_else(|| CommandError::MissingVar(key.to_string()))?;
        out.push_str(value);
        // Advance past `{` + key + `}`.
        i = i + 1 + end_rel + 1;
    }
    Ok(out)
}

fn next_char_boundary(s: &str, idx: usize) -> usize {
    let bytes = s.as_bytes();
    let mut j = idx + 1;
    while j < bytes.len() && !s.is_char_boundary(j) {
        j += 1;
    }
    j
}

/// POSIX-quote a string for safe inclusion in `sh -c` command lines.
/// Wraps the value in single quotes, escaping any embedded apostrophes.
pub fn shell_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

/// Build an augmented PATH so GUI-launched app can find Homebrew + local bins.
fn augmented_path() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let extra = format!("{}/.local/bin:/opt/homebrew/bin:/usr/local/bin", home);
    match std::env::var("PATH") {
        Ok(p) if !p.is_empty() => format!("{}:{}", extra, p),
        _ => extra,
    }
}

/// Run a rendered command via `sh -c`, capture stdout. Used by entry/
/// `*_source` resolvers and by step actions that shell out (e.g. `gh`).
pub fn run_capture(rendered: &str) -> Result<Vec<u8>, CommandError> {
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(rendered)
        .env("PATH", augmented_path())
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        return Err(CommandError::NonZeroExit {
            code: output.status.code(),
            stderr,
        });
    }
    Ok(output.stdout)
}

/// Run + parse stdout as JSON into the requested type. Convenience wrapper
/// over `run_capture` for the common `repo_source` / `issue_source` case.
pub fn run_capture_json<T: serde::de::DeserializeOwned>(
    rendered: &str,
) -> Result<T, CommandError> {
    let bytes = run_capture(rendered)?;
    let parsed = serde_json::from_slice(&bytes)?;
    Ok(parsed)
}

/// Run a rendered command, capture exit code + stdout + stderr. Used by
/// step actions that need to inspect non-zero exits (e.g. `gh issue view`
/// returning 1 when the issue doesn't exist).
pub fn run_capture_full(rendered: &str) -> Result<(i32, Vec<u8>, Vec<u8>), CommandError> {
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(rendered)
        .env("PATH", augmented_path())
        .output()?;
    Ok((
        output.status.code().unwrap_or(-1),
        output.stdout,
        output.stderr,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&'static str, &'static str)]) -> HashMap<&'static str, &'static str> {
        pairs.iter().copied().collect()
    }

    #[test]
    fn render_substitutes_single_var() {
        let out = render_template("gh issue view {issue_number}", &vars(&[("issue_number", "42")]))
            .unwrap();
        assert_eq!(out, "gh issue view 42");
    }

    #[test]
    fn render_substitutes_multiple_vars() {
        let out = render_template(
            "gh issue view {issue_number} --repo {repo}",
            &vars(&[("issue_number", "42"), ("repo", "octo/cat")]),
        )
        .unwrap();
        assert_eq!(out, "gh issue view 42 --repo octo/cat");
    }

    #[test]
    fn render_tolerates_whitespace_inside_placeholder() {
        let out = render_template("hi { repo }!", &vars(&[("repo", "x")])).unwrap();
        assert_eq!(out, "hi x!");
    }

    #[test]
    fn render_passes_through_lone_braces_and_empty_placeholder() {
        let out = render_template("{} { not closed", &vars(&[])).unwrap();
        assert_eq!(out, "{} { not closed");
    }

    #[test]
    fn render_errors_on_missing_var() {
        let err = render_template("hi {missing}", &vars(&[])).unwrap_err();
        match err {
            CommandError::MissingVar(k) => assert_eq!(k, "missing"),
            other => panic!("expected MissingVar, got {other:?}"),
        }
    }

    #[test]
    fn render_doubled_braces_collapse_to_literals() {
        // `{{` → `{` and `}}` → `}` so JSON-emitting commands survive.
        let template = r#"echo '[{{"repo":"{repo}"}}]'"#;
        let out = render_template(template, &vars(&[("repo", "o/r")])).unwrap();
        assert_eq!(out, r#"echo '[{"repo":"o/r"}]'"#);
    }

    #[test]
    fn render_handles_multiline_template() {
        let template = "gh issue list\n  --repo {repo}\n  --limit 100";
        let out = render_template(template, &vars(&[("repo", "octo/cat")])).unwrap();
        assert!(out.contains("--repo octo/cat"));
        assert!(out.contains("--limit 100"));
    }

    #[test]
    fn run_capture_returns_stdout_for_success() {
        let out = run_capture("printf hello").unwrap();
        assert_eq!(out, b"hello");
    }

    #[test]
    fn run_capture_errors_on_non_zero_exit() {
        let err = run_capture("printf err >&2; exit 7").unwrap_err();
        match err {
            CommandError::NonZeroExit { code, stderr } => {
                assert_eq!(code, Some(7));
                assert!(stderr.contains("err"));
            }
            other => panic!("expected NonZeroExit, got {other:?}"),
        }
    }

    #[test]
    fn run_capture_json_decodes_array() {
        let v: Vec<i64> = run_capture_json("printf '[1,2,3]'").unwrap();
        assert_eq!(v, vec![1, 2, 3]);
    }

    #[test]
    fn run_capture_full_returns_exit_and_streams() {
        let (code, out, err) =
            run_capture_full("printf out; printf err >&2; exit 3").unwrap();
        assert_eq!(code, 3);
        assert_eq!(out, b"out");
        assert_eq!(err, b"err");
    }
}

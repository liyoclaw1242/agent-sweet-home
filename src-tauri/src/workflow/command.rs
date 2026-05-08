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
/// Unknown placeholders return `MissingVar`; literal `{{` / `}}` are not
/// special-cased here — entry-level commands don't need brace escaping.
///
/// Whitespace inside the placeholder is allowed (`{ repo }` matches
/// `{repo}`) so multi-line YAML scalars survive a `serde_yaml` reflow.
pub fn render_template(template: &str, vars: &HashMap<&str, &str>) -> Result<String, CommandError> {
    let mut out = String::with_capacity(template.len());
    let mut chars = template.char_indices().peekable();

    while let Some((idx, ch)) = chars.next() {
        if ch != '{' {
            out.push(ch);
            continue;
        }
        // Find the matching '}'. If none, treat the '{' as literal.
        let rest = &template[idx + ch.len_utf8()..];
        let Some(end_rel) = rest.find('}') else {
            out.push(ch);
            continue;
        };
        let key = rest[..end_rel].trim();
        // Skip empty `{}` — pass through literally.
        if key.is_empty() {
            out.push(ch);
            continue;
        }
        let value = vars
            .get(key)
            .ok_or_else(|| CommandError::MissingVar(key.to_string()))?;
        out.push_str(value);
        // Advance the iterator past the closing '}' (idx + '{' + key + '}').
        let consumed = ch.len_utf8() + end_rel + '}'.len_utf8();
        for _ in 0..(consumed - ch.len_utf8()) {
            chars.next();
        }
    }
    Ok(out)
}

/// Run a rendered command via `sh -c`, capture stdout. Used by entry/
/// `*_source` resolvers. Phase 2 fills the body — until then the runtime
/// has no consumer that calls this.
pub fn run_capture(_rendered: &str) -> Result<Vec<u8>, CommandError> {
    todo!("Phase 2: std::process::Command::new(\"sh\").arg(\"-c\").arg(rendered).output()")
}

/// Run + parse stdout as JSON into the requested type. Convenience wrapper
/// over `run_capture` for the common `repo_source` / `issue_source` case.
pub fn run_capture_json<T: serde::de::DeserializeOwned>(
    _rendered: &str,
) -> Result<T, CommandError> {
    todo!("Phase 2: run_capture(rendered).and_then(|bytes| serde_json::from_slice(&bytes))")
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
    fn render_handles_multiline_template() {
        let template = "gh issue list\n  --repo {repo}\n  --limit 100";
        let out = render_template(template, &vars(&[("repo", "octo/cat")])).unwrap();
        assert!(out.contains("--repo octo/cat"));
        assert!(out.contains("--limit 100"));
    }
}

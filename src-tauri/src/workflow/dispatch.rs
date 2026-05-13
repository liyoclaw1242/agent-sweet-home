//! Pure dispatch — given an issue snapshot and an ordered list of rules,
//! return the first matching `Directive`. No I/O, no GitHub, no spawn —
//! the runtime in Phase 4 wraps this with side effects.
//!
//! Predicate semantics (mirror the design § 2 spec):
//! - `has_label`        — exact literal match in `issue.labels`
//! - `matches_label`    — any label starts with the prefix (e.g. `status:`)
//! - `not_has_label`    — sugar for `not: { has_label: ... }`
//! - `issue_state`      — `"open"` or `"closed"`
//! - `has_marker`       — body contains `<!-- key: ... -->`
//! - `expr`             — arbitrary minijinja boolean
//! - `all` / `any` / `not` — boolean combinators

use crate::workflow::expr::{ExprContext, ExprEngine, ExprError};
use crate::workflow::spec::{AtomPredicate, Directive, DispatchRule, Predicate};

#[derive(thiserror::Error, Debug)]
pub enum DispatchError {
    #[error("expression error: {0}")]
    Expr(#[from] ExprError),
}

/// Walk `rules` in order, return the first whose `when` matches. If nothing
/// matches we return `Directive::NoAction { reason: "no rule matched" }` so
/// callers don't have to special-case the empty result.
pub fn dispatch(
    ctx: &ExprContext,
    rules: &[DispatchRule],
    engine: &ExprEngine,
) -> Result<Directive, DispatchError> {
    Ok(dispatch_with_index(ctx, rules, engine)?.0)
}

/// Like `dispatch` but also returns the 0-based index of the matched rule,
/// or `None` when no rule matched. Used by the runtime to populate
/// `dispatch_log.rule_index` for causal-chain tracing.
pub fn dispatch_with_index(
    ctx: &ExprContext,
    rules: &[DispatchRule],
    engine: &ExprEngine,
) -> Result<(Directive, Option<usize>), DispatchError> {
    for (i, rule) in rules.iter().enumerate() {
        if eval_predicate(&rule.when, ctx, engine)? {
            return Ok((rule.then.clone(), Some(i)));
        }
    }
    Ok((
        Directive::NoAction {
            reason: "no rule matched".into(),
        },
        None,
    ))
}

pub fn eval_predicate(
    pred: &Predicate,
    ctx: &ExprContext,
    engine: &ExprEngine,
) -> Result<bool, DispatchError> {
    eval_predicate_inner(pred, ctx, engine, None)
}

/// Pre-spawn variant — resolves `repo_path_exists`, `path_exists`, and `role`
/// atoms that the plain dispatcher can't evaluate. Falls through to the
/// regular evaluator for everything else.
pub fn eval_predicate_with_env(
    pred: &Predicate,
    ctx: &ExprContext,
    engine: &ExprEngine,
    env: &PreSpawnEnv<'_>,
) -> Result<bool, DispatchError> {
    eval_predicate_inner(pred, ctx, engine, Some(env))
}

/// Coordinates the pre-spawn evaluator needs to resolve atoms that are
/// meaningless during dispatch (no role / repo_path resolved yet).
pub struct PreSpawnEnv<'a> {
    pub role: &'a str,
    pub repo_path: &'a std::path::Path,
}

fn eval_predicate_inner(
    pred: &Predicate,
    ctx: &ExprContext,
    engine: &ExprEngine,
    env: Option<&PreSpawnEnv<'_>>,
) -> Result<bool, DispatchError> {
    match pred {
        Predicate::All { all } => {
            for p in all {
                if !eval_predicate_inner(p, ctx, engine, env)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Predicate::Any { any } => {
            for p in any {
                if eval_predicate_inner(p, ctx, engine, env)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Predicate::Not { not } => Ok(!eval_predicate_inner(not, ctx, engine, env)?),
        Predicate::Atom(atom) => Ok(eval_atom(atom, ctx, engine, env)?),
    }
}

fn eval_atom(
    atom: &AtomPredicate,
    ctx: &ExprContext,
    engine: &ExprEngine,
    env: Option<&PreSpawnEnv<'_>>,
) -> Result<bool, DispatchError> {
    Ok(match atom {
        AtomPredicate::HasLabel(name) => ExprEngine::has_label(ctx, name),
        AtomPredicate::MatchesLabel(prefix) => ExprEngine::matches_label(ctx, prefix),
        AtomPredicate::NotHasLabel(name) => !ExprEngine::has_label(ctx, name),
        AtomPredicate::IssueState(state) => ExprEngine::issue_state_is(ctx, state),
        AtomPredicate::HasMarker(key) => ExprEngine::has_marker(ctx, key),
        AtomPredicate::Expr(expr) => engine.eval_bool(expr, ctx)?,
        // Pre-spawn-only atoms: resolve via PreSpawnEnv when present (i.e.
        // when called from `apply_pre_spawn`); fall back to `false` when
        // called from the plain dispatcher so a stray atom in a dispatch
        // rule skips the rule instead of panicking.
        AtomPredicate::RepoPathExists(_) => env
            .map(|e| e.repo_path.exists())
            .unwrap_or(false),
        AtomPredicate::PathExists(template) => env
            .map(|e| {
                std::path::Path::new(
                    &template.replace("{repo_path}", &e.repo_path.to_string_lossy()),
                )
                .exists()
            })
            .unwrap_or(false),
        AtomPredicate::Role(r) => env.map(|e| e.role == r).unwrap_or(false),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::expr::IssueSnapshot;
    use crate::workflow::spec::Workflow;
    use std::collections::HashMap;

    fn snapshot(labels: &[&str]) -> IssueSnapshot {
        IssueSnapshot {
            number: 1,
            title: "t".into(),
            body: "b".into(),
            state: "open".into(),
            labels: labels.iter().map(|s| s.to_string()).collect(),
            markers: HashMap::new(),
        }
    }

    fn engine() -> ExprEngine {
        ExprEngine::new()
    }

    fn rules_from(yaml: &str) -> Vec<DispatchRule> {
        // Entry block is required by the schema; manual mode with a no-op
        // command is the lightest fixture.
        let wrapper = format!(
            r#"version: 1
entry:
  modes: [manual]
  manual:
    issue_source:
      command: "true"
roles: {{}}
dispatch:
  rules:
{}
"#,
            yaml.lines()
                .map(|l| format!("    {l}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        Workflow::from_yaml(&wrapper).unwrap().dispatch.rules
    }

    #[test]
    fn first_matching_rule_wins() {
        let rules = rules_from(
            r#"
- when:
    has_label: needs-be
  then:
    directive: spawn_fresh
    role: be
- when:
    has_label: needs-shape
  then:
    directive: spawn_fresh
    role: shape
"#,
        );
        let ctx = ExprContext::with_issue(snapshot(&["needs-shape"]));
        let d = dispatch(&ctx, &rules, &engine()).unwrap();
        match d {
            Directive::SpawnFresh { role, .. } => assert_eq!(role, "shape"),
            other => panic!("expected SpawnFresh, got {other:?}"),
        }
    }

    #[test]
    fn empty_rules_yield_default_no_action() {
        let ctx = ExprContext::with_issue(snapshot(&[]));
        let d = dispatch(&ctx, &[], &engine()).unwrap();
        assert!(matches!(d, Directive::NoAction { .. }));
    }

    #[test]
    fn all_predicate_requires_every_branch_true() {
        let rules = rules_from(
            r#"
- when:
    all:
      - has_label: ready
      - matches_label: "status:"
  then:
    directive: human_review
"#,
        );
        let ctx_no = ExprContext::with_issue(snapshot(&["ready"]));
        assert!(matches!(
            dispatch(&ctx_no, &rules, &engine()).unwrap(),
            Directive::NoAction { .. }
        ));

        let ctx_yes = ExprContext::with_issue(snapshot(&["ready", "status:in-progress"]));
        assert!(matches!(
            dispatch(&ctx_yes, &rules, &engine()).unwrap(),
            Directive::HumanReview { .. }
        ));
    }

    #[test]
    fn any_predicate_short_circuits_true() {
        let rules = rules_from(
            r#"
- when:
    any:
      - has_label: foo
      - has_label: bar
      - has_label: baz
  then:
    directive: wait
"#,
        );
        let ctx = ExprContext::with_issue(snapshot(&["bar"]));
        assert!(matches!(
            dispatch(&ctx, &rules, &engine()).unwrap(),
            Directive::Wait { .. }
        ));
    }

    #[test]
    fn not_predicate_inverts() {
        let rules = rules_from(
            r#"
- when:
    not:
      has_label: blocked
  then:
    directive: spawn_fresh
    role: shape
"#,
        );
        let ctx_no = ExprContext::with_issue(snapshot(&["blocked"]));
        assert!(matches!(
            dispatch(&ctx_no, &rules, &engine()).unwrap(),
            Directive::NoAction { .. }
        ));
        let ctx_yes = ExprContext::with_issue(snapshot(&["other"]));
        assert!(matches!(
            dispatch(&ctx_yes, &rules, &engine()).unwrap(),
            Directive::SpawnFresh { .. }
        ));
    }

    #[test]
    fn issue_state_and_marker_atoms() {
        let rules = rules_from(
            r#"
- when:
    all:
      - issue_state: open
      - has_marker: spec_version
  then:
    directive: spawn_fresh
    role: be
"#,
        );
        let mut snap = snapshot(&[]);
        snap.markers.insert("spec_version".into(), "1".into());
        let ctx = ExprContext::with_issue(snap);
        assert!(matches!(
            dispatch(&ctx, &rules, &engine()).unwrap(),
            Directive::SpawnFresh { .. }
        ));
    }

    #[test]
    fn expr_predicate_evaluates_minijinja() {
        let rules = rules_from(
            r#"
- when:
    expr: "issue.number > 5"
  then:
    directive: spawn_fresh
    role: shape
"#,
        );
        let ctx_low = ExprContext::with_issue(IssueSnapshot {
            number: 3,
            ..snapshot(&[])
        });
        assert!(matches!(
            dispatch(&ctx_low, &rules, &engine()).unwrap(),
            Directive::NoAction { .. }
        ));
        let ctx_high = ExprContext::with_issue(IssueSnapshot {
            number: 10,
            ..snapshot(&[])
        });
        assert!(matches!(
            dispatch(&ctx_high, &rules, &engine()).unwrap(),
            Directive::SpawnFresh { .. }
        ));
    }

    #[test]
    fn not_has_label_atom_matches_when_label_absent() {
        let rules = rules_from(
            r#"
- when:
    not_has_label: human-review
  then:
    directive: spawn_fresh
    role: shape
"#,
        );
        let ctx = ExprContext::with_issue(snapshot(&["other"]));
        assert!(matches!(
            dispatch(&ctx, &rules, &engine()).unwrap(),
            Directive::SpawnFresh { .. }
        ));
        let ctx_blocked = ExprContext::with_issue(snapshot(&["human-review"]));
        assert!(matches!(
            dispatch(&ctx_blocked, &rules, &engine()).unwrap(),
            Directive::NoAction { .. }
        ));
    }
}

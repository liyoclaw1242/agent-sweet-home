//! On-result handlers + degrade fallback + unblock pass.
//!
//! Phase 1 stub — public API surface is fixed; bodies land in Phase 2 once
//! the GitHub writer + spawn bridge exist. The runtime calls these after
//! every spawn returns (or fails to return) structured output.

use crate::workflow::expr::{ExprContext, ExprEngine, IssueSnapshot};
use crate::workflow::spec::{KindHandler, Step, UnblockConfig, Workflow};

#[derive(thiserror::Error, Debug)]
pub enum ResultError {
    #[error("no on_result handler matched role={role:?} kind={kind:?}")]
    NoHandlerMatched { role: String, kind: String },
    #[error("expression error: {0}")]
    Expr(#[from] crate::workflow::expr::ExprError),
}

/// Apply the on_result handler matching `(role, kind)` against the spawn's
/// structured output. Mutates `ctx.bindings` as `bind:` steps execute, and
/// emits side-effect actions (comments, label edits, sub-issue creation, …)
/// via the Phase-2 GitHub writer.
pub fn apply_on_result(
    _wf: &Workflow,
    _role: &str,
    _kind: &str,
    _out: &serde_json::Value,
    _ctx: &mut ExprContext,
    _engine: &ExprEngine,
) -> Result<(), ResultError> {
    todo!("Phase 2: walk the matched KindHandler.steps via execute_steps()")
}

/// Walk a list of steps, dispatching each control-flow / action variant to
/// its executor. Recursive: `if` / `for_each` re-enter this fn on their
/// inner steps. Single source of truth shared by `apply_on_result`,
/// `apply_pre_spawn`, `apply_unblock`, and `apply_degrade`.
pub fn execute_steps(
    _steps: &[Step],
    _ctx: &mut ExprContext,
    _engine: &ExprEngine,
) -> Result<(), ResultError> {
    todo!("Phase 2: pattern-match each Step → action executor / control-flow walker")
}

/// Degrade fallback — fired by the runtime when a spawn finishes but emits
/// no parseable structured output. Walks `wf.on_no_structured_output.steps`.
pub fn apply_degrade(
    _wf: &Workflow,
    _ctx: &mut ExprContext,
    _engine: &ExprEngine,
) -> Result<(), ResultError> {
    todo!("Phase 2: short-circuit if on_no_structured_output is None; else execute_steps()")
}

/// Look at every `status:blocked` issue, parse `<!-- deps: #X #Y -->` from
/// the body, and promote to `status:ready` once every dep is closed. Runs
/// once per pollOnce(), before dispatch.
pub fn apply_unblock_pass(
    _cfg: &UnblockConfig,
    _issue: &IssueSnapshot,
    _ctx: &mut ExprContext,
    _engine: &ExprEngine,
) -> Result<bool, ResultError> {
    todo!("Phase 2: parse deps marker, check closed-ness, run on_unblock steps if all closed")
}

/// Internal helper used by `apply_on_result` to find the right `KindHandler`
/// for a given (role, kind) pair. Pulled out so it can be unit-tested
/// independently once Phase 2 lands.
#[allow(dead_code)]
fn find_handler<'a>(wf: &'a Workflow, role: &str, kind: &str) -> Option<&'a KindHandler> {
    wf.on_result_for(role)?.iter().find(|h| h.kind == kind)
}

#[cfg(test)]
mod tests {
    // Only one test that exercises the public lookup is needed for Phase 1
    // — the actual execution paths panic via todo!() until Phase 2.

    use super::*;

    #[test]
    fn find_handler_returns_matching_kind() {
        let yaml = r#"
version: 1
entry:
  modes: [manual]
  manual:
    issue_source:
      command: "gh issue view {issue_number} --repo {repo} --json number,title,body,labels,state"
roles: {}
dispatch:
  rules: []
on_result:
  - role: arch-shape
    when: { kind: decomposition }
    steps:
      - add_labels: ["status:done"]
  - role: arch-shape
    when: { kind: rejected }
    steps:
      - add_labels: ["human-review"]
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        assert!(find_handler(&wf, "arch-shape", "decomposition").is_some());
        assert!(find_handler(&wf, "arch-shape", "rejected").is_some());
        assert!(find_handler(&wf, "arch-shape", "needs_consultation").is_none());
        assert!(find_handler(&wf, "implementer", "decomposition").is_none());
    }
}

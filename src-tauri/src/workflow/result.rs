//! On-result handlers + degrade fallback + unblock pass.
//!
//! `apply_on_result` is the entry the runtime calls after each spawn returns
//! parseable structured output: it looks up the right `KindHandler`, then
//! `execute_steps` walks every YAML step, dispatching control flow (If /
//! ForEach / Bind) and actions (CreateIssue / Comment / AddLabels / …).
//!
//! M1 scope: `If`, `Bind` (control flow); `AddLabels`, `RemoveLabels`,
//! `Comment` (actions). `apply_on_result` + `apply_degrade` shells call into
//! `execute_steps`. M2 fills the remaining variants and `apply_unblock_pass`.

use std::path::PathBuf;

use serde_json::json;

use crate::workflow::command::{run_capture, CommandError};
use crate::workflow::expr::{ExprContext, ExprEngine, IssueSnapshot};
use crate::workflow::spec::{
    ActionInput, ActionStep, CommentBody, ControlFlow, ElifBranch, KindHandler, Step, UnblockConfig,
    Workflow,
};

#[derive(thiserror::Error, Debug)]
pub enum ResultError {
    #[error("no on_result handler matched role={role:?} kind={kind:?}")]
    NoHandlerMatched { role: String, kind: String },
    #[error("expression error: {0}")]
    Expr(#[from] crate::workflow::expr::ExprError),
    #[error("github cli failed: {0}")]
    GhCli(#[from] CommandError),
    #[error("unsupported step (will land in M2): {0}")]
    Unsupported(&'static str),
}

/// Repo + filesystem coordinates the runtime hands every step. Held alongside
/// `ExprContext` so action handlers can shell out to `gh` against the right
/// target without threading 8 args through every fn.
#[derive(Debug, Clone)]
pub struct RuntimeContext {
    pub repo_full_name: String,
    pub repo_path: PathBuf,
    /// Issue currently being processed. Used to derive `<num>` for
    /// `gh issue comment / edit` calls. The same data is in
    /// `ExprContext.issue` — we keep a copy here so action handlers can
    /// touch it without going through the engine.
    pub issue_number: u64,
}

impl RuntimeContext {
    pub fn from_issue(repo_full_name: String, repo_path: PathBuf, issue: &IssueSnapshot) -> Self {
        Self {
            repo_full_name,
            repo_path,
            issue_number: issue.number,
        }
    }
}

// ---------------------------------------------------------------------------
// Public entries
// ---------------------------------------------------------------------------

/// Apply the on_result handler matching `(role, kind)` against the spawn's
/// structured output. Mutates `ctx.bindings` as `bind:` steps execute, and
/// emits side-effect actions (comments, label edits, …) via `gh`.
pub fn apply_on_result(
    wf: &Workflow,
    role: &str,
    kind: &str,
    out: &serde_json::Value,
    ctx: &mut ExprContext,
    engine: &ExprEngine,
    rt: &RuntimeContext,
) -> Result<(), ResultError> {
    ctx.out = Some(out.clone());
    let handler = find_handler(wf, role, kind).ok_or_else(|| ResultError::NoHandlerMatched {
        role: role.to_string(),
        kind: kind.to_string(),
    })?;
    execute_steps(&handler.steps, ctx, engine, rt)
}

/// Walk a list of steps, dispatching each control-flow / action variant to
/// its executor. Recursive: `If` re-enters this fn on the matching branch.
/// Single source of truth shared by `apply_on_result`, `apply_pre_spawn`
/// (M2), `apply_unblock` (M2), and `apply_degrade`.
pub fn execute_steps(
    steps: &[Step],
    ctx: &mut ExprContext,
    engine: &ExprEngine,
    rt: &RuntimeContext,
) -> Result<(), ResultError> {
    for step in steps {
        match step {
            Step::Control(cf) => execute_control(cf, ctx, engine, rt)?,
            Step::Action(action) => execute_action(action, ctx, engine, rt)?,
        }
    }
    Ok(())
}

/// Degrade fallback — fired by the runtime when a spawn finishes but emits
/// no parseable structured output. Walks `wf.on_no_structured_output.steps`.
pub fn apply_degrade(
    wf: &Workflow,
    ctx: &mut ExprContext,
    engine: &ExprEngine,
    rt: &RuntimeContext,
) -> Result<(), ResultError> {
    let Some(block) = &wf.on_no_structured_output else {
        return Ok(());
    };
    execute_steps(&block.steps, ctx, engine, rt)
}

/// Look at every `status:blocked` issue, parse `<!-- deps: #X #Y -->` from
/// the body, and promote to `status:ready` once every dep is closed.
///
/// **M2** — the M1 build returns `Ok(false)` so the runtime skeleton can be
/// wired up without the unblock pass yet.
pub fn apply_unblock_pass(
    _cfg: &UnblockConfig,
    _issue: &IssueSnapshot,
    _ctx: &mut ExprContext,
    _engine: &ExprEngine,
) -> Result<bool, ResultError> {
    Ok(false)
}

// ---------------------------------------------------------------------------
// Control flow
// ---------------------------------------------------------------------------

fn execute_control(
    cf: &ControlFlow,
    ctx: &mut ExprContext,
    engine: &ExprEngine,
    rt: &RuntimeContext,
) -> Result<(), ResultError> {
    match cf {
        ControlFlow::If {
            condition,
            steps,
            elif,
            else_steps,
        } => execute_if(condition, steps, elif, else_steps.as_deref(), ctx, engine, rt),
        ControlFlow::ForEach { .. } => Err(ResultError::Unsupported("for_each")),
        ControlFlow::Bind { bind } => execute_bind(bind, ctx, engine),
    }
}

fn execute_if(
    condition: &str,
    steps: &[Step],
    elif: &[ElifBranch],
    else_steps: Option<&[Step]>,
    ctx: &mut ExprContext,
    engine: &ExprEngine,
    rt: &RuntimeContext,
) -> Result<(), ResultError> {
    if engine.eval_bool(condition, ctx)? {
        return execute_steps(steps, ctx, engine, rt);
    }
    for branch in elif {
        if engine.eval_bool(&branch.elif, ctx)? {
            return execute_steps(&branch.steps, ctx, engine, rt);
        }
    }
    if let Some(else_block) = else_steps {
        return execute_steps(else_block, ctx, engine, rt);
    }
    Ok(())
}

fn execute_bind(
    bindings: &std::collections::HashMap<String, String>,
    ctx: &mut ExprContext,
    engine: &ExprEngine,
) -> Result<(), ResultError> {
    for (name, expr) in bindings {
        let value = engine.eval_value(expr, ctx)?;
        ctx.bindings.insert(name.clone(), minijinja_value_to_serde(&value));
    }
    Ok(())
}

/// Best-effort conversion of a minijinja Value back into serde_json::Value
/// so it survives subsequent `eval_value` calls (which round-trip through
/// `serde_to_value`). For complex values we fall back to a JSON parse of
/// the Display form, which minijinja prints as JSON-ish for lists/objects.
fn minijinja_value_to_serde(v: &minijinja::Value) -> serde_json::Value {
    use minijinja::value::ValueKind as K;
    match v.kind() {
        K::Bool => json!(v.is_true()),
        K::Number => {
            if let Some(i) = v.as_i64() {
                json!(i)
            } else {
                let s = v.to_string();
                s.parse::<f64>()
                    .ok()
                    .map(serde_json::Value::from)
                    .unwrap_or_else(|| json!(s))
            }
        }
        K::String => json!(v.as_str().unwrap_or_default().to_string()),
        K::None | K::Undefined => serde_json::Value::Null,
        _ => {
            let s = v.to_string();
            serde_json::from_str(&s).unwrap_or(json!(s))
        }
    }
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

fn execute_action(
    step: &ActionStep,
    ctx: &mut ExprContext,
    engine: &ExprEngine,
    rt: &RuntimeContext,
) -> Result<(), ResultError> {
    match &step.action {
        ActionInput::AddLabels(labels) => action_add_labels(labels, rt, ctx, engine),
        ActionInput::RemoveLabels(labels) => action_remove_labels(labels, rt, ctx, engine),
        ActionInput::Comment(body) => action_comment(body, rt, ctx, engine),

        // M2 surface — explicit Unsupported so the runtime fails loud
        // instead of silently no-op'ing.
        ActionInput::CreateIssue { .. } => Err(ResultError::Unsupported("create_issue")),
        ActionInput::TransitionStatus { .. } => Err(ResultError::Unsupported("transition_status")),
        ActionInput::SetBodyMarker(_) => Err(ResultError::Unsupported("set_body_marker")),
        ActionInput::PushBranchAndPr { .. } => Err(ResultError::Unsupported("push_branch_and_pr")),
        ActionInput::RunCommand { .. } => Err(ResultError::Unsupported("run_command")),
        ActionInput::AbortSpawn(_) => Err(ResultError::Unsupported("abort_spawn (pre-spawn only)")),
        ActionInput::Reroute { .. } => Err(ResultError::Unsupported("reroute (pre-spawn only)")),
    }
}

fn action_add_labels(
    labels: &[String],
    rt: &RuntimeContext,
    ctx: &ExprContext,
    engine: &ExprEngine,
) -> Result<(), ResultError> {
    let rendered = render_each(labels, ctx, engine)?;
    if rendered.is_empty() {
        return Ok(());
    }
    let cmd = format!(
        "gh issue edit {num} --repo {repo} {add}",
        num = rt.issue_number,
        repo = shell_quote(&rt.repo_full_name),
        add = rendered
            .iter()
            .map(|l| format!("--add-label {}", shell_quote(l)))
            .collect::<Vec<_>>()
            .join(" ")
    );
    run_capture(&cmd)?;
    Ok(())
}

fn action_remove_labels(
    labels: &[String],
    rt: &RuntimeContext,
    ctx: &ExprContext,
    engine: &ExprEngine,
) -> Result<(), ResultError> {
    let rendered = render_each(labels, ctx, engine)?;
    if rendered.is_empty() {
        return Ok(());
    }
    let cmd = format!(
        "gh issue edit {num} --repo {repo} {rem}",
        num = rt.issue_number,
        repo = shell_quote(&rt.repo_full_name),
        rem = rendered
            .iter()
            .map(|l| format!("--remove-label {}", shell_quote(l)))
            .collect::<Vec<_>>()
            .join(" ")
    );
    run_capture(&cmd)?;
    Ok(())
}

fn action_comment(
    body: &CommentBody,
    rt: &RuntimeContext,
    ctx: &ExprContext,
    engine: &ExprEngine,
) -> Result<(), ResultError> {
    let text = match body {
        CommentBody::Inline(template) => engine.render(template, ctx)?,
        CommentBody::Detailed { template, body } => match (template, body) {
            (Some(t), _) => engine.render(t, ctx)?,
            (None, Some(b)) => b.clone(),
            (None, None) => return Ok(()),
        },
    };
    let cmd = format!(
        "gh issue comment {num} --repo {repo} --body {body}",
        num = rt.issue_number,
        repo = shell_quote(&rt.repo_full_name),
        body = shell_quote(&text),
    );
    run_capture(&cmd)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn render_each(
    items: &[String],
    ctx: &ExprContext,
    engine: &ExprEngine,
) -> Result<Vec<String>, ResultError> {
    items
        .iter()
        .map(|s| engine.render(s, ctx).map_err(ResultError::from))
        .collect()
}

/// POSIX `sh` single-quote escape. Used for every value we hand to
/// `sh -c "<rendered command>"`.
fn shell_quote(s: &str) -> String {
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

fn find_handler<'a>(wf: &'a Workflow, role: &str, kind: &str) -> Option<&'a KindHandler> {
    wf.on_result_for(role)?.iter().find(|h| h.kind == kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn rt() -> RuntimeContext {
        RuntimeContext {
            repo_full_name: "octo/cat".into(),
            repo_path: "/tmp/x".into(),
            issue_number: 42,
        }
    }

    fn issue() -> IssueSnapshot {
        IssueSnapshot {
            number: 42,
            title: "demo".into(),
            body: "body".into(),
            state: "open".into(),
            labels: vec!["status:ready".into()],
            markers: HashMap::new(),
        }
    }

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
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        assert!(find_handler(&wf, "arch-shape", "decomposition").is_some());
        assert!(find_handler(&wf, "arch-shape", "missing").is_none());
    }

    #[test]
    fn shell_quote_handles_apostrophes() {
        assert_eq!(shell_quote("hi"), "'hi'");
        assert_eq!(shell_quote("it's a test"), "'it'\\''s a test'");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn execute_bind_writes_into_context() {
        let engine = ExprEngine::new();
        let mut ctx = ExprContext::with_issue(issue());
        ctx.out = Some(json!({ "answer": 42 }));
        let mut bindings = HashMap::new();
        bindings.insert("ans".into(), "out.answer".into());
        execute_bind(&bindings, &mut ctx, &engine).unwrap();
        assert_eq!(ctx.bindings.get("ans").and_then(|v| v.as_i64()), Some(42));
    }

    #[test]
    fn execute_if_picks_first_truthy_branch() {
        let yaml = r#"
version: 1
entry:
  modes: [manual]
  manual:
    issue_source:
      command: "gh issue view {issue_number}"
roles: {}
dispatch: { rules: [] }
on_result:
  - role: foo
    when: { kind: bar }
    steps:
      - if: "out.answer == 1"
        steps:
          - add_labels: ["one"]
        elif:
          - elif: "out.answer == 2"
            steps:
              - add_labels: ["two"]
        else:
          - add_labels: ["other"]
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        let engine = ExprEngine::new();
        let mut ctx = ExprContext::with_issue(issue());

        // We can't shell out in unit tests, so verify the branch *would* run
        // by parsing the if/elif/else into the right Step variants.
        let handler = find_handler(&wf, "foo", "bar").unwrap();
        match &handler.steps[0] {
            Step::Control(ControlFlow::If {
                condition,
                steps,
                elif,
                else_steps,
            }) => {
                assert_eq!(condition, "out.answer == 1");
                assert_eq!(steps.len(), 1);
                assert_eq!(elif.len(), 1);
                assert_eq!(elif[0].elif, "out.answer == 2");
                assert!(else_steps.is_some());
            }
            other => panic!("expected If, got {other:?}"),
        }

        // Sanity: predicate evaluator works with our context.
        ctx.out = Some(json!({ "answer": 2 }));
        assert!(engine.eval_bool("out.answer == 2", &ctx).unwrap());
    }

    #[test]
    fn apply_degrade_no_block_returns_ok() {
        let yaml = r#"
version: 1
entry:
  modes: [manual]
  manual:
    issue_source:
      command: "gh issue view {issue_number}"
roles: {}
dispatch: { rules: [] }
on_result: []
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        let engine = ExprEngine::new();
        let mut ctx = ExprContext::with_issue(issue());
        let rtc = rt();
        apply_degrade(&wf, &mut ctx, &engine, &rtc).unwrap();
    }

    #[test]
    fn apply_on_result_no_match_returns_typed_error() {
        let yaml = r#"
version: 1
entry:
  modes: [manual]
  manual:
    issue_source:
      command: "gh issue view {issue_number}"
roles: {}
dispatch: { rules: [] }
on_result: []
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        let engine = ExprEngine::new();
        let mut ctx = ExprContext::with_issue(issue());
        let rtc = rt();
        let err =
            apply_on_result(&wf, "x", "y", &json!({}), &mut ctx, &engine, &rtc).unwrap_err();
        assert!(matches!(err, ResultError::NoHandlerMatched { .. }));
    }
}

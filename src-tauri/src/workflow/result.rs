//! On-result handlers + degrade fallback + unblock pass + pre-spawn
//! evaluator.
//!
//! `apply_on_result` is the entry the runtime calls after each spawn returns
//! parseable structured output: it looks up the right `KindHandler`, then
//! `execute_steps` walks every YAML step, dispatching control flow (If /
//! ForEach / Bind) and actions (CreateIssue / Comment / AddLabels / …).
//!
//! Action implementations shell out to `gh` / `git` via `command::run_capture`.
//! All values that go onto the command line are POSIX-quoted by `shell_quote`.

use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::json;

use crate::workflow::command::{run_capture, run_capture_full, CommandError};
use crate::workflow::expr::{ExprContext, ExprEngine, IssueSnapshot};
use crate::workflow::spec::{
    ActionInput, ActionStep, BoolOrExpr, CommentBody, ControlFlow, DepsValue, ElifBranch,
    KindHandler, LabelEntry, Step, UnblockConfig, Workflow,
};

#[derive(thiserror::Error, Debug)]
pub enum ResultError {
    #[error("no on_result handler matched role={role:?} kind={kind:?}")]
    NoHandlerMatched { role: String, kind: String },
    #[error("expression error: {0}")]
    Expr(#[from] crate::workflow::expr::ExprError),
    #[error("github cli failed: {0}")]
    GhCli(#[from] CommandError),
    #[error("invalid create_issue response: {0}")]
    BadCreateIssue(String),
    #[error("step is pre-spawn-only and may not appear in on_result: {0}")]
    PreSpawnOnly(&'static str),
}

/// Repo + filesystem coordinates the runtime hands every step. Held alongside
/// `ExprContext` so action handlers can shell out to `gh` against the right
/// target without threading 8 args through every fn.
#[derive(Debug, Clone)]
pub struct RuntimeContext {
    pub repo_full_name: String,
    pub repo_path: PathBuf,
    pub issue_number: u64,
    /// Active `for_each` accumulators. Each frame holds the per-iteration
    /// result objects pushed by `commit_iter_result`. The top frame is
    /// surfaced to expressions as `_iter_results`; on `for_each` exit we
    /// pop and stash the popped vector into `_last_iter_results`.
    pub iter_stack: Vec<Vec<serde_json::Value>>,
    /// Set by `CreateIssue` when running inside a `for_each` so the
    /// per-iteration commit can merge it into the iteration's result object.
    pub last_create_issue: Option<serde_json::Value>,
    /// Snapshot of `ctx.bindings` keys at iter start so we can compute the
    /// "new this iter" delta without copying the whole bindings map. Top
    /// frame matches top of `iter_stack`.
    pub iter_binding_keys_at_start: Vec<Vec<String>>,
}

impl RuntimeContext {
    pub fn from_issue(repo_full_name: String, repo_path: PathBuf, issue: &IssueSnapshot) -> Self {
        Self {
            repo_full_name,
            repo_path,
            issue_number: issue.number,
            iter_stack: Vec::new(),
            last_create_issue: None,
            iter_binding_keys_at_start: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Public entries
// ---------------------------------------------------------------------------

pub fn apply_on_result(
    wf: &Workflow,
    role: &str,
    kind: &str,
    out: &serde_json::Value,
    ctx: &mut ExprContext,
    engine: &ExprEngine,
    rt: &mut RuntimeContext,
) -> Result<(), ResultError> {
    ctx.out = Some(out.clone());
    let handler = find_handler(wf, role, kind).ok_or_else(|| ResultError::NoHandlerMatched {
        role: role.to_string(),
        kind: kind.to_string(),
    })?;
    execute_steps(&handler.steps, ctx, engine, rt)
}

pub fn execute_steps(
    steps: &[Step],
    ctx: &mut ExprContext,
    engine: &ExprEngine,
    rt: &mut RuntimeContext,
) -> Result<(), ResultError> {
    for step in steps {
        match step {
            Step::Control(cf) => execute_control(cf, ctx, engine, rt)?,
            Step::Action(action) => execute_action(action, ctx, engine, rt)?,
        }
    }
    Ok(())
}

pub fn apply_degrade(
    wf: &Workflow,
    ctx: &mut ExprContext,
    engine: &ExprEngine,
    rt: &mut RuntimeContext,
) -> Result<(), ResultError> {
    let Some(block) = &wf.on_no_structured_output else {
        return Ok(());
    };
    execute_steps(&block.steps, ctx, engine, rt)
}

/// Look at one `status:blocked` issue and promote to `status:ready` once
/// every dep referenced in `<!-- deps: #X #Y -->` is closed. Returns
/// `Ok(true)` when the issue was promoted (so the caller can `continue` to
/// the next issue without dispatching).
pub fn apply_unblock_pass(
    cfg: &UnblockConfig,
    issue: &IssueSnapshot,
    ctx: &mut ExprContext,
    engine: &ExprEngine,
    rt: &mut RuntimeContext,
) -> Result<bool, ResultError> {
    if !cfg.enabled {
        return Ok(false);
    }
    if !issue.labels.iter().any(|l| l == "status:blocked") {
        return Ok(false);
    }
    let deps = parse_deps_marker(&issue.body);
    if deps.is_empty() {
        return Ok(false);
    }
    for dep in &deps {
        if !is_issue_closed(&rt.repo_full_name, *dep)? {
            return Ok(false);
        }
    }
    // All deps closed → promote this issue.
    let promote_cmd = format!(
        "gh issue edit {num} --repo {repo} --remove-label {blocked} --add-label {ready}",
        num = issue.number,
        repo = shell_quote(&rt.repo_full_name),
        blocked = shell_quote("status:blocked"),
        ready = shell_quote("status:ready"),
    );
    run_capture(&promote_cmd)?;
    if !cfg.on_unblock.is_empty() {
        execute_steps(&cfg.on_unblock, ctx, engine, rt)?;
    }
    Ok(true)
}

// ---------------------------------------------------------------------------
// Pre-spawn evaluator — handles AbortSpawn / Reroute that aren't legal in
// on_result. Returns a side-band Directive override.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum PreSpawnOutcome {
    Continue,
    Abort,
    Reroute { role: String, mode: Option<String> },
}

pub fn apply_pre_spawn(
    hooks: &[crate::workflow::spec::PreSpawnHook],
    ctx: &mut ExprContext,
    engine: &ExprEngine,
    rt: &mut RuntimeContext,
) -> Result<PreSpawnOutcome, ResultError> {
    for hook in hooks {
        if !crate::workflow::dispatch::eval_predicate(&hook.condition, ctx, engine)
            .map_err(|e| ResultError::Expr(map_dispatch_error_to_expr(e)))?
        {
            continue;
        }
        let outcome = execute_pre_spawn_steps(&hook.steps, ctx, engine, rt)?;
        if !matches!(outcome, PreSpawnOutcome::Continue) {
            return Ok(outcome);
        }
    }
    Ok(PreSpawnOutcome::Continue)
}

fn map_dispatch_error_to_expr(e: crate::workflow::dispatch::DispatchError) -> crate::workflow::expr::ExprError {
    crate::workflow::expr::ExprError::NotBoolean(format!("dispatch eval: {e}"))
}

fn execute_pre_spawn_steps(
    steps: &[Step],
    ctx: &mut ExprContext,
    engine: &ExprEngine,
    rt: &mut RuntimeContext,
) -> Result<PreSpawnOutcome, ResultError> {
    for step in steps {
        match step {
            Step::Control(cf) => {
                // Pre-spawn supports the same control flow; recurse via the
                // result-side walker for plain steps and handle the branch
                // outcomes explicitly.
                let outcome = execute_pre_spawn_control(cf, ctx, engine, rt)?;
                if !matches!(outcome, PreSpawnOutcome::Continue) {
                    return Ok(outcome);
                }
            }
            Step::Action(ActionStep { action, .. }) => match action {
                ActionInput::AbortSpawn(true) => return Ok(PreSpawnOutcome::Abort),
                ActionInput::AbortSpawn(false) => {}
                ActionInput::Reroute { role, mode } => {
                    return Ok(PreSpawnOutcome::Reroute {
                        role: role.clone(),
                        mode: mode.clone(),
                    });
                }
                // Side-effect actions (transition_status, comment, …) are
                // legal in pre-spawn — fall through to the normal action
                // executor.
                _ => execute_action_inner(action, None, ctx, engine, rt)?,
            },
        }
    }
    Ok(PreSpawnOutcome::Continue)
}

fn execute_pre_spawn_control(
    cf: &ControlFlow,
    ctx: &mut ExprContext,
    engine: &ExprEngine,
    rt: &mut RuntimeContext,
) -> Result<PreSpawnOutcome, ResultError> {
    match cf {
        ControlFlow::If {
            condition,
            steps,
            elif,
            else_steps,
        } => {
            if engine.eval_bool(condition, ctx)? {
                return execute_pre_spawn_steps(steps, ctx, engine, rt);
            }
            for branch in elif {
                if engine.eval_bool(&branch.elif, ctx)? {
                    return execute_pre_spawn_steps(&branch.steps, ctx, engine, rt);
                }
            }
            if let Some(else_block) = else_steps {
                return execute_pre_spawn_steps(else_block, ctx, engine, rt);
            }
            Ok(PreSpawnOutcome::Continue)
        }
        ControlFlow::Bind { bind } => {
            execute_bind(bind, ctx, engine)?;
            Ok(PreSpawnOutcome::Continue)
        }
        ControlFlow::ForEach { .. } => {
            // for_each in pre-spawn is unusual; treat as continue.
            Ok(PreSpawnOutcome::Continue)
        }
    }
}

// ---------------------------------------------------------------------------
// Control flow
// ---------------------------------------------------------------------------

fn execute_control(
    cf: &ControlFlow,
    ctx: &mut ExprContext,
    engine: &ExprEngine,
    rt: &mut RuntimeContext,
) -> Result<(), ResultError> {
    match cf {
        ControlFlow::If {
            condition,
            steps,
            elif,
            else_steps,
        } => execute_if(condition, steps, elif, else_steps.as_deref(), ctx, engine, rt),
        ControlFlow::ForEach {
            iter_expr,
            var,
            steps,
        } => execute_for_each(iter_expr, var, steps, ctx, engine, rt),
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
    rt: &mut RuntimeContext,
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

fn execute_for_each(
    iter_expr: &str,
    var: &str,
    steps: &[Step],
    ctx: &mut ExprContext,
    engine: &ExprEngine,
    rt: &mut RuntimeContext,
) -> Result<(), ResultError> {
    let value = engine.eval_value(iter_expr, ctx)?;
    let items: Vec<minijinja::Value> = match value.try_iter() {
        Ok(it) => it.collect(),
        Err(_) => return Ok(()),
    };

    rt.iter_stack.push(Vec::new());
    rt.iter_binding_keys_at_start.push(Vec::new());

    let saved_iter_var = ctx.bindings.remove(var);

    for item in items {
        let item_serde = minijinja_value_to_serde(&item);

        // bind iter var
        ctx.bindings.insert(var.to_string(), item_serde.clone());
        // surface current accumulator as `_iter_results` for in-loop expressions
        publish_iter_results(rt, ctx);

        // snapshot current binding keys so we can detect new ones added this iter
        let starting_keys: Vec<String> = ctx.bindings.keys().cloned().collect();
        if let Some(top) = rt.iter_binding_keys_at_start.last_mut() {
            *top = starting_keys.clone();
        }
        rt.last_create_issue = None;

        execute_steps(steps, ctx, engine, rt)?;

        // build per-iteration result: iter_var attrs + new bindings + last create_issue
        let mut entry = serde_json::Map::new();
        if let Some(obj) = item_serde.as_object() {
            for (k, v) in obj {
                entry.insert(k.clone(), v.clone());
            }
        }
        for (k, v) in &ctx.bindings {
            if k == var || k == "_iter_results" || k == "_last_iter_results" {
                continue;
            }
            if !starting_keys.contains(k) {
                entry.insert(k.clone(), v.clone());
            }
        }
        if let Some(ci) = &rt.last_create_issue {
            if let Some(obj) = ci.as_object() {
                for (k, v) in obj {
                    entry.insert(k.clone(), v.clone());
                }
            }
        }
        if let Some(top) = rt.iter_stack.last_mut() {
            top.push(serde_json::Value::Object(entry));
        }
    }

    // pop iter context
    let popped = rt.iter_stack.pop().unwrap_or_default();
    rt.iter_binding_keys_at_start.pop();
    ctx.bindings
        .insert("_last_iter_results".into(), serde_json::Value::Array(popped));
    publish_iter_results(rt, ctx);
    if let Some(prev) = saved_iter_var {
        ctx.bindings.insert(var.to_string(), prev);
    } else {
        ctx.bindings.remove(var);
    }
    rt.last_create_issue = None;

    Ok(())
}

fn publish_iter_results(rt: &RuntimeContext, ctx: &mut ExprContext) {
    if let Some(top) = rt.iter_stack.last() {
        ctx.bindings
            .insert("_iter_results".into(), serde_json::Value::Array(top.clone()));
    } else {
        ctx.bindings.remove("_iter_results");
    }
}

fn execute_bind(
    bindings: &HashMap<String, String>,
    ctx: &mut ExprContext,
    engine: &ExprEngine,
) -> Result<(), ResultError> {
    for (name, expr) in bindings {
        let value = engine.eval_value(expr, ctx)?;
        ctx.bindings
            .insert(name.clone(), minijinja_value_to_serde(&value));
    }
    Ok(())
}

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
        K::Seq => {
            let mut arr = Vec::new();
            if let Ok(iter) = v.try_iter() {
                for item in iter {
                    arr.push(minijinja_value_to_serde(&item));
                }
            }
            serde_json::Value::Array(arr)
        }
        K::Map => {
            let mut map = serde_json::Map::new();
            if let Ok(iter) = v.try_iter() {
                for key in iter {
                    let k_str = key.as_str().map(str::to_string).unwrap_or_else(|| key.to_string());
                    let val = v.get_item(&key).unwrap_or(minijinja::Value::from(()));
                    map.insert(k_str, minijinja_value_to_serde(&val));
                }
            }
            serde_json::Value::Object(map)
        }
        _ => {
            let s = v.to_string();
            serde_json::from_str(&s).unwrap_or(json!(s))
        }
    }
}

// ---------------------------------------------------------------------------
// Action dispatch
// ---------------------------------------------------------------------------

fn execute_action(
    step: &ActionStep,
    ctx: &mut ExprContext,
    engine: &ExprEngine,
    rt: &mut RuntimeContext,
) -> Result<(), ResultError> {
    execute_action_inner(&step.action, step.bind.as_deref(), ctx, engine, rt)
}

fn execute_action_inner(
    action: &ActionInput,
    bind: Option<&str>,
    ctx: &mut ExprContext,
    engine: &ExprEngine,
    rt: &mut RuntimeContext,
) -> Result<(), ResultError> {
    match action {
        ActionInput::AddLabels(labels) => action_add_labels(labels, rt, ctx, engine),
        ActionInput::RemoveLabels(labels) => action_remove_labels(labels, rt, ctx, engine),
        ActionInput::Comment(body) => action_comment(body, rt, ctx, engine),
        ActionInput::CreateIssue {
            title,
            labels,
            body_template,
            body,
            deps,
        } => action_create_issue(
            title,
            labels,
            body_template.as_deref(),
            body.as_deref(),
            deps.as_ref(),
            bind,
            rt,
            ctx,
            engine,
        ),
        ActionInput::TransitionStatus { from, to } => {
            action_transition_status(from.as_deref(), to, rt, ctx, engine)
        }
        ActionInput::SetBodyMarker(map) => action_set_body_marker(map, rt, ctx, engine),
        ActionInput::PushBranchAndPr {
            branch,
            base,
            title,
            body_template,
            closes_issue,
            post_merge_note,
        } => action_push_branch_and_pr(
            branch.as_deref(),
            base,
            title,
            body_template.as_deref(),
            closes_issue,
            post_merge_note.as_deref(),
            rt,
            ctx,
            engine,
        ),
        ActionInput::RunCommand {
            argv,
            cwd,
            stdin,
            bind_stdout,
            bind_exit,
        } => action_run_command(
            argv,
            cwd.as_deref(),
            stdin.as_deref(),
            bind_stdout.as_deref(),
            bind_exit.as_deref(),
            ctx,
            engine,
        ),
        ActionInput::AbortSpawn(_) => Err(ResultError::PreSpawnOnly("abort_spawn")),
        ActionInput::Reroute { .. } => Err(ResultError::PreSpawnOnly("reroute")),
    }
}

// ---------------------------------------------------------------------------
// Action implementations
// ---------------------------------------------------------------------------

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
    let text = render_comment_body(body, ctx, engine)?;
    if text.is_empty() {
        return Ok(());
    }
    let cmd = format!(
        "gh issue comment {num} --repo {repo} --body {body}",
        num = rt.issue_number,
        repo = shell_quote(&rt.repo_full_name),
        body = shell_quote(&text),
    );
    run_capture(&cmd)?;
    Ok(())
}

fn render_comment_body(
    body: &CommentBody,
    ctx: &ExprContext,
    engine: &ExprEngine,
) -> Result<String, ResultError> {
    Ok(match body {
        CommentBody::Inline(template) => engine.render(template, ctx)?,
        CommentBody::Detailed { template, body } => match (template, body) {
            (Some(t), _) => engine.render(t, ctx)?,
            (None, Some(b)) => b.clone(),
            (None, None) => String::new(),
        },
    })
}

#[allow(clippy::too_many_arguments)]
fn action_create_issue(
    title: &str,
    labels: &[LabelEntry],
    body_template: Option<&str>,
    body: Option<&str>,
    deps: Option<&DepsValue>,
    bind: Option<&str>,
    rt: &mut RuntimeContext,
    ctx: &mut ExprContext,
    engine: &ExprEngine,
) -> Result<(), ResultError> {
    let title_rendered = engine.render(title, ctx)?;
    let mut body_rendered = match (body_template, body) {
        (Some(t), _) => engine.render(t, ctx)?,
        (None, Some(b)) => b.to_string(),
        (None, None) => String::new(),
    };
    // Append <!-- deps: -->: marker so unblock_pass can find them later.
    let dep_numbers = resolve_deps(deps, ctx, engine)?;
    if !dep_numbers.is_empty() {
        let marker = format!(
            "\n\n<!-- deps: {} -->",
            dep_numbers
                .iter()
                .map(|n| format!("#{n}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
        body_rendered.push_str(&marker);
    }

    let mut label_args: Vec<String> = Vec::new();
    for entry in labels {
        match entry {
            LabelEntry::Plain(s) => label_args.push(engine.render(s, ctx)?),
            LabelEntry::Conditional { condition, label } => {
                if engine.eval_bool(condition, ctx)? {
                    label_args.push(engine.render(label, ctx)?);
                }
            }
        }
    }

    let mut cmd = format!(
        "gh issue create --repo {repo} --title {title}",
        repo = shell_quote(&rt.repo_full_name),
        title = shell_quote(&title_rendered),
    );
    if !body_rendered.is_empty() {
        cmd.push_str(&format!(" --body {}", shell_quote(&body_rendered)));
    }
    for l in &label_args {
        cmd.push_str(&format!(" --label {}", shell_quote(l)));
    }
    let stdout_bytes = run_capture(&cmd)?;
    let url = String::from_utf8_lossy(&stdout_bytes).trim().to_string();
    let number = url
        .rsplit('/')
        .next()
        .and_then(|s| s.parse::<u64>().ok())
        .ok_or_else(|| ResultError::BadCreateIssue(url.clone()))?;

    let result = json!({
        "number": number,
        "url": url,
        "deps": dep_numbers,
    });

    if let Some(bind_name) = bind {
        ctx.bindings.insert(bind_name.to_string(), result.clone());
    }
    rt.last_create_issue = Some(result);
    Ok(())
}

fn resolve_deps(
    deps: Option<&DepsValue>,
    ctx: &ExprContext,
    engine: &ExprEngine,
) -> Result<Vec<u64>, ResultError> {
    let Some(deps) = deps else {
        return Ok(Vec::new());
    };
    let raw_value = match deps {
        DepsValue::Expr(expr) => engine.eval_value(expr, ctx)?,
        DepsValue::List(items) => {
            let mut acc = Vec::new();
            for it in items {
                acc.push(engine.eval_value(it, ctx)?);
            }
            minijinja::Value::from(acc)
        }
    };
    let serde = minijinja_value_to_serde(&raw_value);
    let arr = match &serde {
        serde_json::Value::Array(a) => a.clone(),
        serde_json::Value::Null => return Ok(Vec::new()),
        other => vec![other.clone()],
    };
    let mut out = Vec::new();
    for v in arr {
        if let Some(n) = v.as_u64() {
            out.push(n);
        } else if let Some(s) = v.as_str() {
            if let Ok(n) = s.trim_start_matches('#').parse::<u64>() {
                out.push(n);
            }
        }
    }
    Ok(out)
}

fn action_transition_status(
    from: Option<&str>,
    to: &str,
    rt: &RuntimeContext,
    ctx: &ExprContext,
    engine: &ExprEngine,
) -> Result<(), ResultError> {
    let to_rendered = engine.render(to, ctx)?;
    let mut cmd = format!(
        "gh issue edit {num} --repo {repo} --add-label {add}",
        num = rt.issue_number,
        repo = shell_quote(&rt.repo_full_name),
        add = shell_quote(&format!("status:{to_rendered}")),
    );
    if let Some(from_name) = from {
        let from_rendered = engine.render(from_name, ctx)?;
        cmd.push_str(&format!(
            " --remove-label {}",
            shell_quote(&format!("status:{from_rendered}"))
        ));
    }
    run_capture(&cmd)?;
    Ok(())
}

fn action_set_body_marker(
    markers: &HashMap<String, String>,
    rt: &RuntimeContext,
    ctx: &ExprContext,
    engine: &ExprEngine,
) -> Result<(), ResultError> {
    if markers.is_empty() {
        return Ok(());
    }
    let view_cmd = format!(
        "gh issue view {num} --repo {repo} --json body --jq .body",
        num = rt.issue_number,
        repo = shell_quote(&rt.repo_full_name),
    );
    let body_bytes = run_capture(&view_cmd)?;
    let mut body = String::from_utf8_lossy(&body_bytes).to_string();

    for (key, value_template) in markers {
        let value = engine.render(value_template, ctx)?;
        body = upsert_marker(&body, key, &value);
    }

    let edit_cmd = format!(
        "gh issue edit {num} --repo {repo} --body {body}",
        num = rt.issue_number,
        repo = shell_quote(&rt.repo_full_name),
        body = shell_quote(&body),
    );
    run_capture(&edit_cmd)?;
    Ok(())
}

fn upsert_marker(body: &str, key: &str, value: &str) -> String {
    let pat = format!("<!-- {key}:");
    let new_marker = format!("<!-- {key}: {value} -->");
    if let Some(start) = body.find(&pat) {
        if let Some(end_rel) = body[start..].find("-->") {
            let end = start + end_rel + "-->".len();
            let mut out = String::new();
            out.push_str(&body[..start]);
            out.push_str(&new_marker);
            out.push_str(&body[end..]);
            return out;
        }
    }
    let mut out = body.to_string();
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push('\n');
    out.push_str(&new_marker);
    out
}

#[allow(clippy::too_many_arguments)]
fn action_push_branch_and_pr(
    branch: Option<&str>,
    base: &str,
    title: &str,
    body_template: Option<&str>,
    closes_issue: &BoolOrExpr,
    post_merge_note: Option<&str>,
    rt: &RuntimeContext,
    ctx: &ExprContext,
    engine: &ExprEngine,
) -> Result<(), ResultError> {
    let branch_name = match branch {
        Some(b) => engine.render(b, ctx)?,
        None => ctx
            .out
            .as_ref()
            .and_then(|o| o.get("branch"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| {
                ResultError::BadCreateIssue("push_branch_and_pr: branch unset and out.branch missing".into())
            })?,
    };
    let base_rendered = engine.render(base, ctx)?;
    let title_rendered = engine.render(title, ctx)?;
    let mut body_rendered = match body_template {
        Some(t) => engine.render(t, ctx)?,
        None => String::new(),
    };
    if let Some(note) = post_merge_note {
        body_rendered.push_str("\n\n");
        body_rendered.push_str(&engine.render(note, ctx)?);
    }
    let closes = match closes_issue {
        BoolOrExpr::Bool(b) => *b,
        BoolOrExpr::Expr(e) => engine.eval_bool(e, ctx)?,
    };
    if closes {
        body_rendered.push_str(&format!("\n\nCloses #{}\n", rt.issue_number));
    }

    let push_cmd = format!(
        "cd {cwd} && git push -u origin {branch}",
        cwd = shell_quote(&rt.repo_path.to_string_lossy()),
        branch = shell_quote(&branch_name),
    );
    run_capture(&push_cmd)?;

    let pr_cmd = format!(
        "gh pr create --repo {repo} --base {base} --head {head} --title {title} --body {body}",
        repo = shell_quote(&rt.repo_full_name),
        base = shell_quote(&base_rendered),
        head = shell_quote(&branch_name),
        title = shell_quote(&title_rendered),
        body = shell_quote(&body_rendered),
    );
    run_capture(&pr_cmd)?;
    Ok(())
}

fn action_run_command(
    argv: &[String],
    cwd: Option<&str>,
    stdin: Option<&str>,
    bind_stdout: Option<&str>,
    bind_exit: Option<&str>,
    ctx: &mut ExprContext,
    engine: &ExprEngine,
) -> Result<(), ResultError> {
    let _ = stdin; // M2 doesn't pipe stdin; argv-only invocations are sufficient for current YAML.
    let rendered_argv: Vec<String> = argv
        .iter()
        .map(|a| engine.render(a, ctx))
        .collect::<Result<Vec<_>, _>>()?;
    let cwd_clause = match cwd {
        Some(c) => {
            let rendered = engine.render(c, ctx)?;
            format!("cd {} && ", shell_quote(&rendered))
        }
        None => String::new(),
    };
    let cmd = format!(
        "{cwd}{argv}",
        cwd = cwd_clause,
        argv = rendered_argv
            .iter()
            .map(|a| shell_quote(a))
            .collect::<Vec<_>>()
            .join(" "),
    );
    let (code, stdout, _stderr) = run_capture_full(&cmd)?;
    if let Some(name) = bind_stdout {
        let s = String::from_utf8_lossy(&stdout).into_owned();
        ctx.bindings.insert(name.to_string(), json!(s));
    }
    if let Some(name) = bind_exit {
        ctx.bindings.insert(name.to_string(), json!(code));
    }
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

fn parse_deps_marker(body: &str) -> Vec<u64> {
    let mut deps = Vec::new();
    let needle = "<!-- deps:";
    let Some(start) = body.find(needle) else {
        return deps;
    };
    let after = &body[start + needle.len()..];
    let Some(end) = after.find("-->") else {
        return deps;
    };
    let inner = &after[..end];
    for tok in inner.split(|c: char| c.is_whitespace() || c == ',') {
        let t = tok.trim().trim_start_matches('#');
        if let Ok(n) = t.parse::<u64>() {
            deps.push(n);
        }
    }
    deps
}

fn is_issue_closed(repo: &str, num: u64) -> Result<bool, ResultError> {
    let cmd = format!(
        "gh issue view {num} --repo {repo} --json state --jq .state",
        num = num,
        repo = shell_quote(repo)
    );
    let bytes = run_capture(&cmd)?;
    let state = String::from_utf8_lossy(&bytes).trim().to_uppercase();
    Ok(state == "CLOSED")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt() -> RuntimeContext {
        RuntimeContext {
            repo_full_name: "octo/cat".into(),
            repo_path: "/tmp/x".into(),
            issue_number: 42,
            iter_stack: Vec::new(),
            last_create_issue: None,
            iter_binding_keys_at_start: Vec::new(),
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
    fn shell_quote_handles_apostrophes() {
        assert_eq!(shell_quote("hi"), "'hi'");
        assert_eq!(shell_quote("it's a test"), "'it'\\''s a test'");
    }

    #[test]
    fn parse_deps_marker_collects_numbers() {
        let body = "Some text\n<!-- deps: #12 #13 #14 -->\nmore";
        assert_eq!(parse_deps_marker(body), vec![12, 13, 14]);
        let none = "no marker here";
        assert!(parse_deps_marker(none).is_empty());
    }

    #[test]
    fn upsert_marker_replaces_existing() {
        let body = "hello\n<!-- subdomain: old -->\nworld";
        let out = upsert_marker(body, "subdomain", "new");
        assert!(out.contains("<!-- subdomain: new -->"));
        assert!(!out.contains("old"));
    }

    #[test]
    fn upsert_marker_appends_when_absent() {
        let body = "hello world";
        let out = upsert_marker(body, "k", "v");
        assert!(out.contains("<!-- k: v -->"));
        assert!(out.starts_with("hello world"));
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
    fn for_each_publishes_iter_results_and_last() {
        // We can't actually run create_issue without gh, but we can verify
        // the iter machinery handles a no-op body and the iter var binding
        // round-trips cleanly.
        let engine = ExprEngine::new();
        let mut ctx = ExprContext::with_issue(issue());
        ctx.out = Some(json!({ "items": [{"k": "a"}, {"k": "b"}] }));
        let mut rtc = rt();

        // Simulated steps: just bind a derived var per iter.
        let yaml = r#"
- bind:
    derived: "task.k ~ '!'"
"#;
        let inner_steps: Vec<Step> = serde_yaml::from_str(yaml).unwrap();
        execute_for_each("out.items", "task", &inner_steps, &mut ctx, &engine, &mut rtc).unwrap();
        let last = ctx
            .bindings
            .get("_last_iter_results")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap();
        assert_eq!(last.len(), 2);
        assert_eq!(last[0].get("k").and_then(|v| v.as_str()), Some("a"));
        assert_eq!(last[0].get("derived").and_then(|v| v.as_str()), Some("a!"));
        // iter var no longer in bindings after exit
        assert!(!ctx.bindings.contains_key("task"));
        // _iter_results popped
        assert!(!ctx.bindings.contains_key("_iter_results"));
    }

    #[test]
    fn formatdep_filter_works_via_engine() {
        let engine = ExprEngine::new();
        let mut ctx = ExprContext::with_issue(issue());
        ctx.bindings.insert("nums".into(), json!([1, 2, 3]));
        let s = engine
            .render("{{ nums | map('formatdep') | join(', ') }}", &ctx)
            .unwrap();
        assert_eq!(s, "#1, #2, #3");
    }

    #[test]
    fn lookup_iter_result_number_resolves_index() {
        let engine = ExprEngine::new();
        let mut ctx = ExprContext::with_issue(issue());
        ctx.bindings.insert(
            "_iter_results".into(),
            json!([{"number": 100}, {"number": 200}]),
        );
        let s = engine
            .render("{{ 1 | lookup_iter_result_number }}", &ctx)
            .unwrap();
        assert_eq!(s, "200");
    }

    #[test]
    fn apply_unblock_pass_disabled_returns_false() {
        let cfg = UnblockConfig::default();
        let engine = ExprEngine::new();
        let mut ctx = ExprContext::with_issue(issue());
        let mut rtc = rt();
        assert!(!apply_unblock_pass(&cfg, &issue(), &mut ctx, &engine, &mut rtc).unwrap());
    }

    /// Smoke-load the real production workflow YAML to confirm every step
    /// variant we now support actually deserializes against the agent-team
    /// spec we're targeting. Skipped when run outside the monorepo.
    #[test]
    fn load_real_agent_team_workflow_yaml() {
        let candidates = [
            "../../agent-team/agent-team-v2.workflow.yaml",
            "../../../agent-team/agent-team-v2.workflow.yaml",
        ];
        let Some(path) = candidates.iter().find(|p| std::path::Path::new(p).exists()) else {
            eprintln!("skip: real workflow.yaml not found in known relative paths");
            return;
        };
        let text = std::fs::read_to_string(path).unwrap();
        let wf = Workflow::from_yaml(&text).expect("real workflow yaml must parse");
        // Every role we ship spawn for must have a budget and a system_prompt_file.
        for (role, cfg) in &wf.roles {
            assert!(cfg.budget_usd > 0.0, "{role} budget unset");
            assert!(!cfg.system_prompt_file.is_empty(), "{role} prompt file unset");
        }
        // arch-shape must have at least one decomposition handler.
        let kinds: Vec<&str> = wf
            .on_result_for("arch-shape")
            .map(|hs| hs.iter().map(|h| h.kind.as_str()).collect())
            .unwrap_or_default();
        assert!(kinds.contains(&"decomposition"), "arch-shape kinds: {kinds:?}");
    }
}

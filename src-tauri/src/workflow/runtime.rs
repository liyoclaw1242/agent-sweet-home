//! Workflow runtime — the orchestrator that ties dispatch + spawn + on_result
//! into one async pipeline.
//!
//! Lifetime: one `WorkflowRuntime` is constructed at app start (in `lib.rs`)
//! and held inside an `Arc`. Entry-mode drivers (poll / manual / webhook)
//! borrow it to run dispatch loops. Each dispatch is independent — no shared
//! mutable state between issues, so the runtime can fan out per-issue tasks
//! freely.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;
use tauri::AppHandle;

use crate::db::Db;
use crate::one_shot::OneShotState;
use crate::workflow::dispatch::{dispatch, DispatchError};
use crate::workflow::entry::RepoRef;
use crate::workflow::expr::{ExprContext, ExprEngine, IssueSnapshot};
use crate::workflow::result::{
    apply_degrade, apply_on_result, apply_pre_spawn, apply_unblock_pass, PreSpawnOutcome,
    ResultError, RuntimeContext,
};
use crate::workflow::spawn::{run_spawn, SpawnError, SpawnRequest, SpawnedAgent};
use crate::workflow::spec::{Directive, Workflow};
use crate::workflow::worktree::{allocate as allocate_worktree, cleanup as cleanup_worktree, WorktreeAllocation, WorktreeError};

#[derive(thiserror::Error, Debug)]
pub enum RuntimeError {
    #[error("dispatch: {0}")]
    Dispatch(#[from] DispatchError),
    #[error("spawn: {0}")]
    Spawn(#[from] SpawnError),
    #[error("result handler: {0}")]
    Result(#[from] ResultError),
    #[error("unknown role in directive: {0}")]
    UnknownRole(String),
    #[error("expression: {0}")]
    Expr(#[from] crate::workflow::expr::ExprError),
    #[error("worktree: {0}")]
    Worktree(#[from] WorktreeError),
}

/// Runtime-side outcome of dispatching one issue. Useful to surface in
/// HTTP responses and Tauri events for the UI.
#[derive(Debug, Clone)]
pub enum DispatchOutcome {
    NoAction { reason: String },
    HumanReview { reason: String },
    Wait { reason: String },
    Aborted { role: String },
    Spawned { role: String, status: String, kind: Option<String> },
    Unblocked,
}

pub struct WorkflowRuntime {
    pub wf: Workflow,
    pub workflow_dir: PathBuf,
    pub app: AppHandle,
    pub db: Db,
    pub one_shot: OneShotState,
}

impl WorkflowRuntime {
    pub fn new(
        wf: Workflow,
        workflow_dir: PathBuf,
        app: AppHandle,
        db: Db,
        one_shot: OneShotState,
    ) -> Self {
        Self {
            wf,
            workflow_dir,
            app,
            db,
            one_shot,
        }
    }

    /// Process a single (repo, issue) pair end-to-end. Returns the outcome
    /// so callers (HTTP handler, manual entry, poll loop) can log / emit.
    pub async fn dispatch_one(
        self: &Arc<Self>,
        repo: &RepoRef,
        issue: IssueSnapshot,
    ) -> Result<DispatchOutcome, RuntimeError> {
        let engine = ExprEngine::new();
        let mut ctx = ExprContext::with_issue(issue.clone());
        let mut rt = RuntimeContext::from_issue(
            repo.repo.clone(),
            PathBuf::from(&repo.path),
            &issue,
        );

        // 0. Unblock pass — promote status:blocked issues whose deps are
        //    closed; skip dispatch if we promoted (it'll be picked up next
        //    tick under status:ready).
        if apply_unblock_pass(
            &self.wf.unblock_pass,
            &issue,
            &mut ctx,
            &engine,
            &mut rt,
        )? {
            return Ok(DispatchOutcome::Unblocked);
        }

        // 1. Dispatch — find the first matching rule and read its directive.
        let directive = dispatch(&ctx, &self.wf.dispatch.rules, &engine)?;

        match directive {
            Directive::NoAction { reason } => Ok(DispatchOutcome::NoAction { reason }),
            Directive::Wait { reason } => Ok(DispatchOutcome::Wait { reason }),
            Directive::HumanReview { reason } => Ok(DispatchOutcome::HumanReview { reason }),
            Directive::SpawnFresh { role, mode, .. } => {
                self.run_spawn_pipeline(repo, issue, role, mode, ctx, engine, rt)
                    .await
            }
        }
    }

    async fn run_spawn_pipeline(
        self: &Arc<Self>,
        repo: &RepoRef,
        issue: IssueSnapshot,
        role: String,
        mode: Option<String>,
        mut ctx: ExprContext,
        engine: ExprEngine,
        mut rt: RuntimeContext,
    ) -> Result<DispatchOutcome, RuntimeError> {
        // 2. Pre-spawn evaluator — may abort or reroute. Pass the dispatched
        //    role so `role:` and `repo_path_exists:` atoms can resolve.
        let outcome = apply_pre_spawn(&self.wf.pre_spawn, &role, &mut ctx, &engine, &mut rt)?;
        let (final_role, final_mode) = match outcome {
            PreSpawnOutcome::Abort => return Ok(DispatchOutcome::Aborted { role }),
            PreSpawnOutcome::Reroute { role: r, mode: m } => (r, m),
            PreSpawnOutcome::Continue => (role, mode),
        };

        let role_cfg = self
            .wf
            .roles
            .get(&final_role)
            .ok_or_else(|| RuntimeError::UnknownRole(final_role.clone()))?;

        // 3. If the role wants write isolation, carve a fresh worktree off
        //    the canonical clone before the spawn. Cleanup is best-effort
        //    on the way out, regardless of whether on_result succeeded.
        let canonical_repo_path = PathBuf::from(&repo.path);
        let worktree = if role_cfg.needs_worktree {
            let alloc = allocate_worktree(&canonical_repo_path, issue.number, None)?;
            // Surface the carved branch + worktree path so YAML templates
            // (push_branch_and_pr's body, comments, …) can reference them.
            ctx.bindings
                .insert("branch".into(), Value::String(alloc.branch.clone()));
            ctx.bindings.insert(
                "worktree_path".into(),
                Value::String(alloc.worktree_path.to_string_lossy().into_owned()),
            );
            // Action handlers (push_branch_and_pr / run_command with
            // implicit cwd) follow rt.repo_path — point that at the
            // worktree so push happens from there.
            rt.repo_path = alloc.worktree_path.clone();
            Some(alloc)
        } else {
            None
        };

        let spawn_cwd = worktree
            .as_ref()
            .map(|w| w.worktree_path.as_path())
            .unwrap_or(canonical_repo_path.as_path());

        // 4. Spawn `claude -p`, wait for completion, parse final structured
        //    output (None when the agent emitted no parseable JSON tail).
        let spawn_req = SpawnRequest {
            role: &final_role,
            mode: final_mode.as_deref(),
            role_cfg,
            repo_full_name: &repo.repo,
            // repo_id is best-effort: 0 is fine for the workflow runtime
            // path because the DB row is keyed by run_id, not repo_id.
            repo_id: 0,
            repo_path: spawn_cwd,
            workflow_dir: &self.workflow_dir,
            issue: &issue,
            prompt_header: &build_prompt_header(
                &final_role,
                final_mode.as_deref(),
                &issue,
                worktree.as_ref(),
            ),
        };
        let spawn_result = run_spawn(
            self.app.clone(),
            self.db.clone(),
            self.one_shot.clone(),
            spawn_req,
        )
        .await;

        let agent: SpawnedAgent = match spawn_result {
            Ok(a) => a,
            Err(e) => {
                if let Some(alloc) = &worktree {
                    cleanup_worktree(alloc);
                }
                return Err(RuntimeError::Spawn(e));
            }
        };

        // 5. Translate structured output into GitHub side effects via
        //    on_result handlers; fall back to on_no_structured_output when
        //    the agent didn't emit parseable JSON.
        // Bind `spawn` so degrade / on_result templates can reference cost,
        // role, and status without erroring on undefined values.
        ctx.bindings.insert(
            "spawn".into(),
            serde_json::json!({
                "role": final_role,
                "status": agent.status,
                "exit_code": agent.exit_code,
                "cost_usd": agent.total_cost_usd.unwrap_or(0.0),
                "duration_ms": 0,
                "end_reason": agent.status,
                "last_assistant_text": "",
                "stderr": "",
            }),
        );
        let on_result_outcome = match agent.structured_output.as_ref().and_then(extract_kind) {
            Some(k) => {
                let result = apply_on_result(
                    &self.wf,
                    &final_role,
                    &k,
                    agent.structured_output.as_ref().unwrap_or(&Value::Null),
                    &mut ctx,
                    &engine,
                    &mut rt,
                );
                result.map(|()| Some(k))
            }
            None => apply_degrade(&self.wf, &mut ctx, &engine, &mut rt).map(|()| None),
        };

        // 6. Always tear down the worktree, even if on_result errored —
        //    leaving stale worktrees blocks the next spawn for the same
        //    issue.
        if let Some(alloc) = &worktree {
            cleanup_worktree(alloc);
        }

        let kind_str = on_result_outcome?;

        Ok(DispatchOutcome::Spawned {
            role: final_role,
            status: agent.status,
            kind: kind_str,
        })
    }
}

fn extract_kind(v: &Value) -> Option<String> {
    v.get("kind").and_then(|k| k.as_str()).map(str::to_string)
}

/// Bog-standard prompt header — the orchestrator prepends this to the
/// issue body before handing the prompt to claude. Mirrors the supervisor's
/// per-spawn header convention. When the agent runs inside a worktree, the
/// header surfaces the carved branch + worktree path so the agent can echo
/// them in its structured output without re-deriving from `git`.
pub fn build_prompt_header(
    role: &str,
    mode: Option<&str>,
    issue: &IssueSnapshot,
    worktree: Option<&WorktreeAllocation>,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("ROLE: {role}\n"));
    if let Some(m) = mode {
        out.push_str(&format!("MODE: {m}\n"));
    }
    out.push_str(&format!(
        "ISSUE: #{num} ({state})\nTITLE: {title}\n",
        num = issue.number,
        state = issue.state,
        title = issue.title,
    ));
    if let Some(w) = worktree {
        out.push_str(&format!(
            "WORKTREE: {path}\nBRANCH: {branch}\n",
            path = w.worktree_path.display(),
            branch = w.branch,
        ));
    }
    out
}

/// Parse an issue body for `<!-- key: value -->` markers. Public so entry/*
/// drivers can use it when they receive a raw gh-formatted issue.
pub fn parse_body_markers(body: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut rest = body;
    while let Some(start) = rest.find("<!--") {
        rest = &rest[start + "<!--".len()..];
        let Some(end) = rest.find("-->") else { break };
        let inner = rest[..end].trim();
        rest = &rest[end + "-->".len()..];
        if let Some((k, v)) = inner.split_once(':') {
            let key = k.trim().to_string();
            let val = v.trim().to_string();
            if !key.is_empty() && !key.chars().any(|c| c.is_whitespace()) {
                out.insert(key, val);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(num: u64, title: &str, state: &str) -> IssueSnapshot {
        IssueSnapshot {
            number: num,
            title: title.into(),
            body: String::new(),
            state: state.into(),
            labels: Vec::new(),
            markers: HashMap::new(),
        }
    }

    #[test]
    fn build_prompt_header_includes_role_mode_issue() {
        let h = build_prompt_header("arch-shape", Some("audit"), &issue(7, "Demo", "open"), None);
        assert!(h.contains("ROLE: arch-shape"));
        assert!(h.contains("MODE: audit"));
        assert!(h.contains("ISSUE: #7 (open)"));
        assert!(h.contains("TITLE: Demo"));
        assert!(!h.contains("BRANCH:"));
    }

    #[test]
    fn build_prompt_header_omits_mode_clause_when_none() {
        let h = build_prompt_header("implementer", None, &issue(1, "x", "open"), None);
        assert!(!h.contains("MODE:"));
        assert!(h.contains("ROLE: implementer"));
    }

    #[test]
    fn build_prompt_header_surfaces_worktree_when_present() {
        let alloc = WorktreeAllocation {
            repo_root: PathBuf::from("/tmp/repo"),
            worktree_path: PathBuf::from("/tmp/repo-worktrees/spawn-1-99"),
            branch: "spawn-1-99".into(),
        };
        let h = build_prompt_header("implementer", None, &issue(1, "x", "open"), Some(&alloc));
        assert!(h.contains("WORKTREE: /tmp/repo-worktrees/spawn-1-99"));
        assert!(h.contains("BRANCH: spawn-1-99"));
    }

    #[test]
    fn parse_body_markers_extracts_known_keys() {
        let body = r#"
Some prose.

<!-- parent: #12 -->
<!-- subdomain: billing -->
<!-- deps: #1 #2 -->
<!-- intake-kind: business -->

trailing text
"#;
        let m = parse_body_markers(body);
        assert_eq!(m.get("parent").map(String::as_str), Some("#12"));
        assert_eq!(m.get("subdomain").map(String::as_str), Some("billing"));
        assert_eq!(m.get("deps").map(String::as_str), Some("#1 #2"));
        assert_eq!(m.get("intake-kind").map(String::as_str), Some("business"));
    }

    #[test]
    fn parse_body_markers_ignores_non_marker_comments() {
        let body = "<!-- some narrative comment -->\n<!-- key: val -->";
        let m = parse_body_markers(body);
        assert!(!m.contains_key("some narrative comment"));
        assert_eq!(m.get("key").map(String::as_str), Some("val"));
    }

    #[test]
    fn extract_kind_pulls_string_field() {
        assert_eq!(
            extract_kind(&serde_json::json!({"kind": "decomposition"})),
            Some("decomposition".to_string())
        );
        assert_eq!(extract_kind(&serde_json::json!({})), None);
        assert_eq!(extract_kind(&serde_json::json!({"kind": 42})), None);
    }
}

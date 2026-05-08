//! Workflow runtime — the orchestrator that ties dispatch + spawn + on_result
//! into one async pipeline.
//!
//! Lifetime: one `WorkflowRuntime` is constructed at app start (in `lib.rs`)
//! and held inside an `Arc`. Entry-mode drivers (poll / manual / webhook)
//! borrow it to run dispatch loops. Each dispatch is independent — no shared
//! mutable state between issues, so the runtime can fan out per-issue tasks
//! freely.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
        // 2. Pre-spawn evaluator — may abort or reroute.
        let outcome = apply_pre_spawn(&self.wf.pre_spawn, &mut ctx, &engine, &mut rt)?;
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

        // 3. Spawn `claude -p`, wait for completion, parse final structured
        //    output (None when the agent emitted no parseable JSON tail).
        let spawn_req = SpawnRequest {
            role: &final_role,
            mode: final_mode.as_deref(),
            role_cfg,
            repo_full_name: &repo.repo,
            // repo_id is best-effort: 0 is fine for the workflow runtime
            // path because the DB row is keyed by run_id, not repo_id.
            repo_id: 0,
            repo_path: Path::new(&repo.path),
            workflow_dir: &self.workflow_dir,
            issue: &issue,
            prompt_header: &build_prompt_header(&final_role, final_mode.as_deref(), &issue),
        };
        let agent: SpawnedAgent = run_spawn(
            self.app.clone(),
            self.db.clone(),
            self.one_shot.clone(),
            spawn_req,
        )
        .await?;

        // 4. Translate structured output into GitHub side effects via
        //    on_result handlers; fall back to on_no_structured_output when
        //    the agent didn't emit parseable JSON.
        let kind_str = match agent.structured_output.as_ref().and_then(extract_kind) {
            Some(k) => {
                apply_on_result(
                    &self.wf,
                    &final_role,
                    &k,
                    agent.structured_output.as_ref().unwrap_or(&Value::Null),
                    &mut ctx,
                    &engine,
                    &mut rt,
                )?;
                Some(k)
            }
            None => {
                apply_degrade(&self.wf, &mut ctx, &engine, &mut rt)?;
                None
            }
        };

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
/// per-spawn header convention.
pub fn build_prompt_header(role: &str, mode: Option<&str>, issue: &IssueSnapshot) -> String {
    let mode_clause = mode.map(|m| format!("MODE: {m}\n")).unwrap_or_default();
    format!(
        "ROLE: {role}\n{mode_clause}ISSUE: #{num} ({state})\nTITLE: {title}\n",
        role = role,
        num = issue.number,
        state = issue.state,
        title = issue.title,
    )
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
        let h = build_prompt_header("arch-shape", Some("audit"), &issue(7, "Demo", "open"));
        assert!(h.contains("ROLE: arch-shape"));
        assert!(h.contains("MODE: audit"));
        assert!(h.contains("ISSUE: #7 (open)"));
        assert!(h.contains("TITLE: Demo"));
    }

    #[test]
    fn build_prompt_header_omits_mode_clause_when_none() {
        let h = build_prompt_header("implementer", None, &issue(1, "x", "open"));
        assert!(!h.contains("MODE:"));
        assert!(h.contains("ROLE: implementer"));
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

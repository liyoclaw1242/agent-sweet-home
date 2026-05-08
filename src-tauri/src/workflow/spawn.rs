//! Bridge between the workflow runtime and `one_shot::start_run`.
//!
//! Given a (role, mode, issue, repo) tuple plus the role's `RoleConfig` from
//! the YAML, build the `RunArgs` that one_shot expects, kick off the spawn,
//! poll the SQLite-backed run row until it leaves `running`, then read back
//! the captured log to extract the agent's final structured-JSON output.
//!
//! The runtime is async (tokio), but `one_shot::start_run` is sync and uses
//! its own `std::thread`s for stdout/stderr readers; we wrap the sync calls
//! in `tokio::task::spawn_blocking` so we don't block the async executor.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tokio::task;

use crate::db::Db;
use crate::one_shot::{
    get_run_inner, list_log_lines_inner, start_run, LogLine, OneShotState, RunArgs, RunInfo,
};
use crate::workflow::expr::IssueSnapshot;
use crate::workflow::spec::RoleConfig;

#[derive(thiserror::Error, Debug)]
pub enum SpawnError {
    #[error("system prompt file not found: {0}")]
    PromptFileMissing(PathBuf),
    #[error("read system prompt {0}: {1}")]
    PromptFileRead(PathBuf, std::io::Error),
    #[error("start_run failed: {0}")]
    Start(String),
    #[error("db poll failed: {0}")]
    DbPoll(String),
    #[error("join failed: {0}")]
    Join(#[from] task::JoinError),
    #[error("run vanished from db (id={0})")]
    RunVanished(String),
}

/// Outcome of one ephemeral `claude -p` invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnedAgent {
    pub run_id: String,
    /// `"completed"` | `"failed"` | `"killed"`.
    pub status: String,
    pub exit_code: i32,
    /// Parsed JSON of the agent's final structured output, if any. Sourced
    /// from the last `{"type":"result","subtype":"success","result":"..."}`
    /// line: we take `.result` (the agent's final assistant text) and try to
    /// parse it as JSON. Returns `None` on parse failure or missing result.
    pub structured_output: Option<serde_json::Value>,
    /// Raw `.result` text from the final result event, regardless of whether
    /// it parsed as JSON. Roles whose contract is "emit markdown" (advisors)
    /// surface their advice here and degrade-side handlers echo it as a
    /// comment.
    pub last_assistant_text: Option<String>,
    pub total_cost_usd: Option<f64>,
    /// Wallclock duration of the run in milliseconds, derived from the DB
    /// row (ended_at − started_at). Surfaced for degrade templates.
    pub duration_ms: Option<i64>,
}

/// Parameters captured per-spawn. Lifted out of the call signature so callers
/// can build the struct via field init and avoid 12-arg drift.
pub struct SpawnRequest<'a> {
    pub role: &'a str,
    pub mode: Option<&'a str>,
    pub role_cfg: &'a RoleConfig,
    pub repo_full_name: &'a str,
    pub repo_id: i64,
    pub repo_path: &'a Path,
    /// Directory containing the workflow YAML — used to resolve relative
    /// `system_prompt_file` / `add_dirs` entries.
    pub workflow_dir: &'a Path,
    pub issue: &'a IssueSnapshot,
    /// Header text the orchestrator prepends to the prompt (mode banner,
    /// parent issue ref, dependency markers, …). Always followed by the
    /// issue body.
    pub prompt_header: &'a str,
}

/// Build the `RunArgs` we'll hand to `one_shot::start_run`. Pure function so
/// it can be unit-tested without touching tokio / sqlite / the filesystem.
/// The system prompt content is loaded once at the call site and passed in
/// here as a fully-resolved string.
pub fn build_run_args(
    req: &SpawnRequest,
    system_prompt: String,
    repo_path_str: String,
    add_dirs_resolved: Vec<String>,
) -> RunArgs {
    let (allowed_tools, disallowed_tools, model, budget_usd) =
        resolved_mode_overrides(req.role_cfg, req.mode);

    let prompt = if req.prompt_header.is_empty() {
        req.issue.body.clone()
    } else {
        format!("{}\n\n{}", req.prompt_header, req.issue.body)
    };

    RunArgs {
        repo_id: req.repo_id,
        repo_name: req.repo_full_name.to_string(),
        cwd: repo_path_str,
        prompt,
        model,
        output_format: Some("stream-json".into()),
        permission_mode: None,
        skip_permissions: true,
        effort: None,
        verbose: true,
        include_partial_messages: false,
        system_prompt: Some(system_prompt),
        append_system_prompt: None,
        add_dir: add_dirs_resolved,
        allowed_tools,
        disallowed_tools,
        tools: None,
        agent: None,
        max_budget_usd: Some(budget_usd),
        mcp_config: vec![],
        strict_mcp_config: false,
        resume: None,
        continue_last: false,
        fork_session: false,
        name: Some(format!(
            "{}-{}-issue{}",
            req.role,
            req.mode.unwrap_or("default"),
            req.issue.number
        )),
        extra_args: vec![],
    }
}

/// Spawn a single ephemeral agent + wait for it to finish. Polling cadence
/// is 500 ms — fast enough for short spawns, cheap enough for long ones.
pub async fn run_spawn(
    app: AppHandle,
    db: Db,
    state: OneShotState,
    req: SpawnRequest<'_>,
) -> Result<SpawnedAgent, SpawnError> {
    // 1. Resolve filesystem-bound bits up front so the async section sees
    //    only owned data.
    let prompt_path = req.workflow_dir.join(&req.role_cfg.system_prompt_file);
    if !prompt_path.exists() {
        return Err(SpawnError::PromptFileMissing(prompt_path));
    }
    let system_prompt = std::fs::read_to_string(&prompt_path)
        .map_err(|e| SpawnError::PromptFileRead(prompt_path.clone(), e))?;

    let repo_path_str = req.repo_path.to_string_lossy().to_string();
    let add_dirs_resolved = req
        .role_cfg
        .add_dirs
        .iter()
        .map(|d| {
            d.replace("{repo}", req.repo_full_name)
                .replace("{repo_path}", &repo_path_str)
        })
        .collect::<Vec<_>>();

    let run_args = build_run_args(&req, system_prompt, repo_path_str, add_dirs_resolved);

    // 2. Kick off the spawn (blocking under the hood — start_run threads
    //    out the readers + waiter itself, returning RunInfo immediately).
    let info: RunInfo = {
        let app = app.clone();
        let db = db.clone();
        let state = state.clone();
        task::spawn_blocking(move || start_run(&app, &db, &state, run_args))
            .await?
            .map_err(SpawnError::Start)?
    };

    // 3. Poll the run row until it leaves `running`.
    let run_id = info.id.clone();
    let final_run = wait_until_done(db.clone(), run_id.clone()).await?;

    // 4. Read the captured log + extract the agent's final JSON output.
    let log_lines = {
        let db = db.clone();
        let id = run_id.clone();
        task::spawn_blocking(move || -> Result<Vec<LogLine>, String> {
            let conn = db.0.lock().map_err(|e| e.to_string())?;
            list_log_lines_inner(&conn, &id, -1, 1_000_000).map_err(|e| e.to_string())
        })
        .await?
        .map_err(SpawnError::DbPoll)?
    };

    let last_assistant_text = extract_last_assistant_text(&log_lines);
    let structured_output = extract_structured_output(&log_lines);
    let duration_ms = final_run
        .ended_at
        .map(|end| (end - final_run.started_at).saturating_mul(1000));

    Ok(SpawnedAgent {
        run_id: final_run.id,
        status: final_run.status,
        exit_code: final_run.exit_code.unwrap_or(-1),
        structured_output,
        last_assistant_text,
        total_cost_usd: final_run.total_cost_usd,
        duration_ms,
    })
}

async fn wait_until_done(db: Db, run_id: String) -> Result<RunInfo, SpawnError> {
    loop {
        let db_clone = db.clone();
        let id = run_id.clone();
        let snapshot = task::spawn_blocking(move || -> Result<Option<RunInfo>, String> {
            let conn = db_clone.0.lock().map_err(|e| e.to_string())?;
            get_run_inner(&conn, &id).map_err(|e| e.to_string())
        })
        .await?
        .map_err(SpawnError::DbPoll)?;
        match snapshot {
            None => return Err(SpawnError::RunVanished(run_id)),
            Some(run) if run.status != "running" => return Ok(run),
            Some(_) => tokio::time::sleep(Duration::from_millis(500)).await,
        }
    }
}

fn resolved_mode_overrides(
    cfg: &RoleConfig,
    mode: Option<&str>,
) -> (Vec<String>, Vec<String>, Option<String>, f64) {
    let mut allowed = cfg.allowed_tools.clone();
    let mut disallowed = cfg.disallowed_tools.clone();
    let mut model = cfg.model.clone();
    let mut budget = cfg.budget_usd;
    if let Some(name) = mode {
        if let Some(over) = cfg.mode_overrides.get(name) {
            if !over.allowed_tools.is_empty() {
                allowed = over.allowed_tools.clone();
            }
            if !over.disallowed_tools.is_empty() {
                disallowed = over.disallowed_tools.clone();
            }
            if let Some(m) = &over.model {
                model = Some(m.clone());
            }
            if let Some(b) = over.budget_usd {
                budget = b;
            }
        }
    }
    (allowed, disallowed, model, budget)
}

/// Walk the captured log backwards for the final result event and return
/// `.result` verbatim — regardless of whether it parses as JSON. Roles that
/// emit markdown (advisors) carry their advice through this field.
fn extract_last_assistant_text(log: &[LogLine]) -> Option<String> {
    for line in log.iter().rev() {
        if line.stream != "stdout" {
            continue;
        }
        let parsed: serde_json::Value = match serde_json::from_str(&line.text) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if parsed.get("type").and_then(|v| v.as_str()) != Some("result") {
            continue;
        }
        if let Some(s) = parsed.get("result").and_then(|v| v.as_str()) {
            return Some(s.to_string());
        }
    }
    None
}

/// Walk the captured log backwards looking for the last `result` event from
/// claude. The agent's final assistant text lives in `.result`; we try to
/// parse it as JSON. If it isn't JSON, return `None` — the runtime treats
/// that as "no structured output" and fires `on_no_structured_output`.
fn extract_structured_output(log: &[LogLine]) -> Option<serde_json::Value> {
    for line in log.iter().rev() {
        if line.stream != "stdout" {
            continue;
        }
        let parsed: serde_json::Value = match serde_json::from_str(&line.text) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if parsed.get("type").and_then(|v| v.as_str()) != Some("result") {
            continue;
        }
        let result_text = parsed.get("result").and_then(|v| v.as_str())?;
        return strip_json_fence(result_text)
            .and_then(|inner| serde_json::from_str::<serde_json::Value>(inner).ok())
            .or_else(|| serde_json::from_str::<serde_json::Value>(result_text).ok());
    }
    None
}

/// Strip ```json fences if present. Agents sometimes wrap their final JSON
/// in a markdown code fence; we accept either form.
fn strip_json_fence(s: &str) -> Option<&str> {
    let trimmed = s.trim();
    let after_open = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))?;
    let inner = after_open.trim_start_matches('\n');
    inner.strip_suffix("```").map(str::trim).or(Some(inner))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::spec::ModeOverride;
    use std::collections::HashMap;

    fn role_cfg() -> RoleConfig {
        RoleConfig {
            system_prompt_file: ".claude/agents/foo.md".into(),
            add_dirs: vec!["{repo_path}/.claude/skills/foo".into()],
            allowed_tools: vec!["Read".into(), "Bash(git *)".into()],
            disallowed_tools: vec!["Edit".into()],
            json_schema_file: None,
            budget_usd: 2.0,
            model: Some("sonnet".into()),
            needs_worktree: false,
            mode_overrides: HashMap::from([(
                "review".into(),
                ModeOverride {
                    allowed_tools: vec!["Read".into()],
                    disallowed_tools: vec!["Edit".into(), "Write".into()],
                    model: None,
                    budget_usd: Some(0.5),
                },
            )]),
        }
    }

    fn issue() -> IssueSnapshot {
        IssueSnapshot {
            number: 7,
            title: "demo".into(),
            body: "issue body content".into(),
            state: "open".into(),
            labels: vec!["agent:foo".into(), "status:ready".into()],
            markers: HashMap::new(),
        }
    }

    fn log(text: &str, stream: &str) -> LogLine {
        LogLine {
            run_id: "r".into(),
            seq: 0,
            ts: 0,
            stream: stream.into(),
            text: text.into(),
        }
    }

    #[test]
    fn build_run_args_default_mode_uses_role_defaults() {
        let cfg = role_cfg();
        let req = SpawnRequest {
            role: "foo",
            mode: None,
            role_cfg: &cfg,
            repo_full_name: "octo/cat",
            repo_id: 1,
            repo_path: Path::new("/tmp/x"),
            workflow_dir: Path::new("/tmp"),
            issue: &issue(),
            prompt_header: "MODE: default",
        };
        let args = build_run_args(&req, "you are foo".into(), "/tmp/x".into(), vec![]);
        assert_eq!(args.system_prompt.as_deref(), Some("you are foo"));
        assert_eq!(args.allowed_tools, vec!["Read", "Bash(git *)"]);
        assert_eq!(args.max_budget_usd, Some(2.0));
        assert_eq!(args.model.as_deref(), Some("sonnet"));
        assert!(args.skip_permissions);
        assert!(args.prompt.starts_with("MODE: default"));
        assert!(args.prompt.contains("issue body content"));
    }

    #[test]
    fn build_run_args_review_mode_applies_overrides() {
        let cfg = role_cfg();
        let req = SpawnRequest {
            role: "foo",
            mode: Some("review"),
            role_cfg: &cfg,
            repo_full_name: "octo/cat",
            repo_id: 1,
            repo_path: Path::new("/tmp/x"),
            workflow_dir: Path::new("/tmp"),
            issue: &issue(),
            prompt_header: "",
        };
        let args = build_run_args(&req, "sp".into(), "/tmp/x".into(), vec![]);
        assert_eq!(args.allowed_tools, vec!["Read"]);
        assert_eq!(args.disallowed_tools, vec!["Edit", "Write"]);
        assert_eq!(args.max_budget_usd, Some(0.5));
        // empty header → prompt is exactly the issue body
        assert_eq!(args.prompt, "issue body content");
    }

    #[test]
    fn extract_structured_output_parses_plain_json_result() {
        let lines = vec![
            log(r#"{"type":"system","subtype":"init"}"#, "stdout"),
            log(
                r#"{"type":"result","subtype":"success","result":"{\"kind\":\"decomposition\",\"child_tasks\":[]}"}"#,
                "stdout",
            ),
        ];
        let v = extract_structured_output(&lines).unwrap();
        assert_eq!(v.get("kind").and_then(|x| x.as_str()), Some("decomposition"));
    }

    #[test]
    fn extract_structured_output_strips_markdown_fence() {
        let fenced = "```json\n{\"kind\":\"x\"}\n```";
        let result_event = serde_json::json!({
            "type": "result",
            "subtype": "success",
            "result": fenced,
        });
        let lines = vec![log(&result_event.to_string(), "stdout")];
        let v = extract_structured_output(&lines).unwrap();
        assert_eq!(v.get("kind").and_then(|x| x.as_str()), Some("x"));
    }

    #[test]
    fn extract_structured_output_returns_none_when_no_result_event() {
        let lines = vec![log(r#"{"type":"assistant","message":"hi"}"#, "stdout")];
        assert!(extract_structured_output(&lines).is_none());
    }

    #[test]
    fn extract_structured_output_returns_none_when_result_is_plain_text() {
        let lines = vec![log(
            r#"{"type":"result","subtype":"success","result":"just some prose"}"#,
            "stdout",
        )];
        assert!(extract_structured_output(&lines).is_none());
    }
}

use crate::db::Db;
use crate::terminal::make_session_id;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, State};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunArgs {
    pub repo_id: i64,
    pub repo_name: String,
    pub cwd: String,
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub output_format: Option<String>, // text | json | stream-json
    #[serde(default)]
    pub permission_mode: Option<String>,
    #[serde(default)]
    pub skip_permissions: bool,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default = "default_true")]
    pub verbose: bool,
    #[serde(default)]
    pub include_partial_messages: bool,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub append_system_prompt: Option<String>,
    #[serde(default)]
    pub add_dir: Vec<String>,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub disallowed_tools: Vec<String>,
    #[serde(default)]
    pub tools: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub max_budget_usd: Option<f64>,
    #[serde(default)]
    pub mcp_config: Vec<String>,
    #[serde(default)]
    pub strict_mcp_config: bool,
    #[serde(default)]
    pub resume: Option<String>,
    #[serde(default)]
    pub continue_last: bool,
    #[serde(default)]
    pub fork_session: bool,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub extra_args: Vec<String>,
}

impl Default for RunArgs {
    fn default() -> Self {
        Self {
            repo_id: 0,
            repo_name: String::new(),
            cwd: String::new(),
            prompt: String::new(),
            model: None,
            output_format: None,
            permission_mode: None,
            skip_permissions: false,
            effort: None,
            verbose: true,
            include_partial_messages: false,
            system_prompt: None,
            append_system_prompt: None,
            add_dir: vec![],
            allowed_tools: vec![],
            disallowed_tools: vec![],
            tools: None,
            agent: None,
            max_budget_usd: None,
            mcp_config: vec![],
            strict_mcp_config: false,
            resume: None,
            continue_last: false,
            fork_session: false,
            name: None,
            extra_args: vec![],
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInfo {
    pub id: String,
    pub repo_id: i64,
    pub repo_name: String,
    pub cwd: String,
    pub prompt: String,
    pub argv: Vec<String>,
    pub status: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub exit_code: Option<i32>,
    pub total_cost_usd: Option<f64>,
    pub output_format: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    pub run_id: String,
    pub seq: i64,
    pub ts: i64,
    pub stream: String,
    pub text: String,
}

#[derive(Default, Clone)]
pub struct OneShotState {
    children: Arc<Mutex<HashMap<String, Arc<Mutex<Child>>>>>,
}

impl OneShotState {
    pub fn new() -> Self {
        Self {
            children: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn is_running(&self, id: &str) -> bool {
        self.children
            .lock()
            .map(|m| m.contains_key(id))
            .unwrap_or(false)
    }

    pub fn take_child(&self, id: &str) -> Option<Arc<Mutex<Child>>> {
        self.children.lock().ok()?.remove(id)
    }
}

fn unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn unix_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn expand_tilde(p: &str) -> PathBuf {
    if let Some(stripped) = p.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(stripped);
        }
    }
    if p == "~" {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home);
        }
    }
    PathBuf::from(p)
}

/// Build the argv we hand to `Command::new`. The first element is the
/// program name; everything after that is a flag or positional. Pure
/// function so we can unit-test without spawning.
pub fn build_argv(args: &RunArgs) -> Vec<String> {
    let mut argv: Vec<String> = vec!["claude".into(), "-p".into()];

    let output_format = args
        .output_format
        .clone()
        .unwrap_or_else(|| "stream-json".into());
    argv.push("--output-format".into());
    argv.push(output_format.clone());

    if args.verbose {
        argv.push("--verbose".into());
    }
    if args.include_partial_messages && output_format == "stream-json" {
        argv.push("--include-partial-messages".into());
    }

    if let Some(m) = &args.model {
        argv.push("--model".into());
        argv.push(m.clone());
    }
    if let Some(pm) = &args.permission_mode {
        argv.push("--permission-mode".into());
        argv.push(pm.clone());
    }
    if args.skip_permissions {
        argv.push("--dangerously-skip-permissions".into());
    }
    if let Some(e) = &args.effort {
        argv.push("--effort".into());
        argv.push(e.clone());
    }
    if let Some(sp) = &args.system_prompt {
        argv.push("--system-prompt".into());
        argv.push(sp.clone());
    }
    if let Some(asp) = &args.append_system_prompt {
        argv.push("--append-system-prompt".into());
        argv.push(asp.clone());
    }
    for d in &args.add_dir {
        argv.push("--add-dir".into());
        argv.push(d.clone());
    }
    if !args.allowed_tools.is_empty() {
        argv.push("--allowedTools".into());
        argv.push(args.allowed_tools.join(","));
    }
    if !args.disallowed_tools.is_empty() {
        argv.push("--disallowedTools".into());
        argv.push(args.disallowed_tools.join(","));
    }
    if let Some(t) = &args.tools {
        argv.push("--tools".into());
        argv.push(t.clone());
    }
    if let Some(a) = &args.agent {
        argv.push("--agent".into());
        argv.push(a.clone());
    }
    if let Some(b) = args.max_budget_usd {
        argv.push("--max-budget-usd".into());
        argv.push(format!("{}", b));
    }
    for m in &args.mcp_config {
        argv.push("--mcp-config".into());
        argv.push(m.clone());
    }
    if args.strict_mcp_config {
        argv.push("--strict-mcp-config".into());
    }
    if let Some(r) = &args.resume {
        argv.push("--resume".into());
        argv.push(r.clone());
    }
    if args.continue_last {
        argv.push("--continue".into());
    }
    if args.fork_session {
        argv.push("--fork-session".into());
    }
    if let Some(n) = &args.name {
        argv.push("--name".into());
        argv.push(n.clone());
    }

    for extra in &args.extra_args {
        argv.push(extra.clone());
    }

    if !args.prompt.is_empty() {
        argv.push(args.prompt.clone());
    }

    argv
}

fn insert_run(conn: &Connection, info: &RunInfo) -> rusqlite::Result<()> {
    let argv_json = serde_json::to_string(&info.argv).unwrap_or_else(|_| "[]".into());
    conn.execute(
        "INSERT INTO one_shot_runs
         (id, repo_id, repo_name, cwd, argv_json, prompt, status, started_at, ended_at, exit_code, total_cost_usd, output_format)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, NULL, NULL, ?9)",
        params![
            info.id,
            info.repo_id,
            info.repo_name,
            info.cwd,
            argv_json,
            info.prompt,
            info.status,
            info.started_at,
            info.output_format,
        ],
    )?;
    Ok(())
}

fn append_log_line(
    conn: &Connection,
    run_id: &str,
    stream: &str,
    text: &str,
) -> rusqlite::Result<i64> {
    let next_seq: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(seq), -1) + 1 FROM one_shot_log_lines WHERE run_id = ?1",
            params![run_id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let ts = unix_millis();
    conn.execute(
        "INSERT INTO one_shot_log_lines (run_id, seq, ts, stream, text) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![run_id, next_seq, ts, stream, text],
    )?;
    Ok(next_seq)
}

fn finalize_run(
    conn: &Connection,
    run_id: &str,
    status: &str,
    exit_code: Option<i32>,
    total_cost: Option<f64>,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE one_shot_runs
         SET status = ?1, ended_at = ?2, exit_code = ?3, total_cost_usd = COALESCE(?4, total_cost_usd)
         WHERE id = ?5",
        params![status, unix_secs(), exit_code, total_cost, run_id],
    )?;
    Ok(())
}

fn parse_total_cost(line: &str) -> Option<f64> {
    // claude -p --output-format stream-json emits a final result event
    // containing { type: "result", total_cost_usd: <num>, ... }.
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    if v.get("type")?.as_str()? != "result" {
        return None;
    }
    v.get("total_cost_usd")
        .and_then(|x| x.as_f64())
        .or_else(|| v.get("cost_usd").and_then(|x| x.as_f64()))
}

pub fn list_runs_inner(
    conn: &Connection,
    repo_id: Option<i64>,
    status: Option<&str>,
) -> rusqlite::Result<Vec<RunInfo>> {
    let mut sql = String::from(
        "SELECT id, repo_id, repo_name, cwd, argv_json, prompt, status, started_at, ended_at, exit_code, total_cost_usd, output_format
         FROM one_shot_runs",
    );
    let mut clauses: Vec<String> = vec![];
    if repo_id.is_some() {
        clauses.push("repo_id = ?".into());
    }
    if status.is_some() {
        clauses.push("status = ?".into());
    }
    if !clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&clauses.join(" AND "));
    }
    sql.push_str(" ORDER BY started_at DESC");

    let mut stmt = conn.prepare(&sql)?;
    let mut idx = 1;
    if let Some(id) = repo_id {
        stmt.raw_bind_parameter(idx, id)?;
        idx += 1;
    }
    if let Some(s) = status {
        stmt.raw_bind_parameter(idx, s)?;
    }
    let rows = stmt.raw_query().mapped(|row| {
        let argv_json: String = row.get(4)?;
        let argv: Vec<String> = serde_json::from_str(&argv_json).unwrap_or_default();
        Ok(RunInfo {
            id: row.get(0)?,
            repo_id: row.get(1)?,
            repo_name: row.get(2)?,
            cwd: row.get(3)?,
            argv,
            prompt: row.get(5)?,
            status: row.get(6)?,
            started_at: row.get(7)?,
            ended_at: row.get(8)?,
            exit_code: row.get(9)?,
            total_cost_usd: row.get(10)?,
            output_format: row.get(11)?,
        })
    });
    let mut out = vec![];
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn get_run_inner(conn: &Connection, id: &str) -> rusqlite::Result<Option<RunInfo>> {
    let mut stmt = conn.prepare(
        "SELECT id, repo_id, repo_name, cwd, argv_json, prompt, status, started_at, ended_at, exit_code, total_cost_usd, output_format
         FROM one_shot_runs WHERE id = ?1",
    )?;
    stmt.query_row(params![id], |row| {
        let argv_json: String = row.get(4)?;
        let argv: Vec<String> = serde_json::from_str(&argv_json).unwrap_or_default();
        Ok(RunInfo {
            id: row.get(0)?,
            repo_id: row.get(1)?,
            repo_name: row.get(2)?,
            cwd: row.get(3)?,
            argv,
            prompt: row.get(5)?,
            status: row.get(6)?,
            started_at: row.get(7)?,
            ended_at: row.get(8)?,
            exit_code: row.get(9)?,
            total_cost_usd: row.get(10)?,
            output_format: row.get(11)?,
        })
    })
    .optional()
}

pub fn list_log_lines_inner(
    conn: &Connection,
    run_id: &str,
    since_seq: i64,
    limit: i64,
) -> rusqlite::Result<Vec<LogLine>> {
    let mut stmt = conn.prepare(
        "SELECT run_id, seq, ts, stream, text FROM one_shot_log_lines
         WHERE run_id = ?1 AND seq > ?2 ORDER BY seq ASC LIMIT ?3",
    )?;
    let rows = stmt
        .query_map(params![run_id, since_seq, limit], |row| {
            Ok(LogLine {
                run_id: row.get(0)?,
                seq: row.get(1)?,
                ts: row.get(2)?,
                stream: row.get(3)?,
                text: row.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[tauri::command]
pub fn one_shot_start(
    app: AppHandle,
    db: State<'_, Db>,
    state: State<'_, OneShotState>,
    args: RunArgs,
) -> Result<RunInfo, String> {
    start_run(&app, db.inner(), state.inner(), args)
}

/// Free function for callers that don't have the Tauri State wrappers
/// (HTTP handler, tests). Same behavior as `one_shot_start`.
pub fn start_run(
    app: &AppHandle,
    db: &Db,
    state: &OneShotState,
    args: RunArgs,
) -> Result<RunInfo, String> {
    let cwd_buf = expand_tilde(&args.cwd);
    if !cwd_buf.exists() {
        return Err(format!("cwd does not exist: {}", cwd_buf.display()));
    }
    let argv = build_argv(&args);
    let id = make_session_id(&args.repo_name);
    let started_at = unix_secs();
    let output_format = args
        .output_format
        .clone()
        .unwrap_or_else(|| "stream-json".into());

    let info = RunInfo {
        id: id.clone(),
        repo_id: args.repo_id,
        repo_name: args.repo_name.clone(),
        cwd: cwd_buf.to_string_lossy().to_string(),
        prompt: args.prompt.clone(),
        argv: argv.clone(),
        status: "running".into(),
        started_at,
        ended_at: None,
        exit_code: None,
        total_cost_usd: None,
        output_format,
    };

    {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        insert_run(&conn, &info).map_err(|e| e.to_string())?;
    }

    // Resolve the spawn binary. `build_argv` hardcodes "claude" as argv[0],
    // which works on POSIX (the shim in PATH is invoked directly) but
    // fails on Windows where npm-installed CLIs ship as `.cmd` batch
    // shims (CreateProcess can't execute them directly). Allow override
    // via `CLAUDE_BINARY` env so deployments can point at the underlying
    // `.exe` (e.g. `…/npm/node_modules/@anthropic-ai/claude-code/bin/claude.exe`).
    let prog = std::env::var("CLAUDE_BINARY").unwrap_or_else(|_| argv[0].clone());
    let mut cmd = Command::new(&prog);
    cmd.args(&argv[1..]);
    cmd.current_dir(&cwd_buf);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    if let Some(home) = std::env::var_os("HOME") {
        cmd.env("HOME", home);
    }
    {
        let home = std::env::var("HOME").unwrap_or_default();
        let extra = format!(
            "{}/.local/bin:/opt/homebrew/bin:/usr/local/bin",
            home
        );
        let path = match std::env::var("PATH") {
            Ok(p) if !p.is_empty() => format!("{}:{}", extra, p),
            _ => extra,
        };
        cmd.env("PATH", path);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let conn = db.0.lock().map_err(|e| e.to_string())?;
            let _ = append_log_line(&conn, &id, "stderr", &format!("spawn failed: {e}"));
            let _ = finalize_run(&conn, &id, "failed", Some(-1), None);
            return Err(format!("spawn failed: {e}"));
        }
    };

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "missing stdout".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "missing stderr".to_string())?;
    let child_arc = Arc::new(Mutex::new(child));
    {
        let mut map = state
            .children
            .lock()
            .map_err(|_| "state poisoned".to_string())?;
        map.insert(id.clone(), child_arc.clone());
    }

    let app_for_threads = app.clone();
    let db_for_threads = db.0.clone();
    let id_for_threads = id.clone();
    let state_for_threads = state.children.clone();

    spawn_reader_thread(
        app_for_threads.clone(),
        db_for_threads.clone(),
        id_for_threads.clone(),
        "stdout",
        stdout,
    );
    spawn_reader_thread(
        app_for_threads.clone(),
        db_for_threads.clone(),
        id_for_threads.clone(),
        "stderr",
        stderr,
    );

    std::thread::spawn(move || {
        let exit_code = match child_arc.lock() {
            Ok(mut g) => match g.wait() {
                Ok(status) => status.code().unwrap_or(-1),
                Err(_) => -1,
            },
            Err(_) => -1,
        };
        let status = if exit_code == 0 { "completed" } else { "failed" };
        if let Ok(conn) = db_for_threads.lock() {
            // total_cost is updated in-line as we see the result event;
            // we just close the row here.
            let _ = finalize_run(&conn, &id_for_threads, status, Some(exit_code), None);
        }
        if let Ok(mut map) = state_for_threads.lock() {
            map.remove(&id_for_threads);
        }
        let payload = serde_json::json!({
            "exitCode": exit_code,
            "status": status,
        });
        let _ = app_for_threads.emit(&format!("oneshot:exit:{id_for_threads}"), payload);

        // Eagerly parse stream-json events into run_events after the reader
        // threads have had time to flush their last lines to SQLite.
        let db_parse = db_for_threads.clone();
        let id_parse = id_for_threads.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(500));
            if let Ok(conn) = db_parse.lock() {
                let _ = crate::graph::parse_and_store_run_events(&conn, &id_parse);
            }
        });
    });

    Ok(info)
}

fn spawn_reader_thread<R: std::io::Read + Send + 'static>(
    app: AppHandle,
    db: Arc<Mutex<Connection>>,
    run_id: String,
    stream_name: &'static str,
    reader: R,
) {
    std::thread::spawn(move || {
        let mut buf = BufReader::new(reader);
        let mut line = String::new();
        loop {
            line.clear();
            match buf.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let trimmed = line.trim_end_matches(['\n', '\r']);
                    let cost = parse_total_cost(trimmed);
                    let mut emitted_seq = -1i64;
                    if let Ok(conn) = db.lock() {
                        if let Ok(seq) = append_log_line(&conn, &run_id, stream_name, trimmed) {
                            emitted_seq = seq;
                        }
                        if let Some(c) = cost {
                            let _ = conn.execute(
                                "UPDATE one_shot_runs SET total_cost_usd = ?1 WHERE id = ?2",
                                params![c, &run_id],
                            );
                        }
                    }
                    if emitted_seq >= 0 {
                        let payload = LogLine {
                            run_id: run_id.clone(),
                            seq: emitted_seq,
                            ts: unix_millis(),
                            stream: stream_name.into(),
                            text: trimmed.to_string(),
                        };
                        let _ = app.emit(&format!("oneshot:line:{run_id}"), payload);
                    }
                }
                Err(_) => break,
            }
        }
    });
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ListArgs {
    #[serde(default)]
    pub repo_id: Option<i64>,
    #[serde(default)]
    pub status: Option<String>,
}

#[tauri::command]
pub fn one_shot_list(db: State<'_, Db>, args: ListArgs) -> Result<Vec<RunInfo>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    list_runs_inner(&conn, args.repo_id, args.status.as_deref()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn one_shot_get(db: State<'_, Db>, id: String) -> Result<Option<RunInfo>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    get_run_inner(&conn, &id).map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogArgs {
    pub id: String,
    #[serde(default = "default_since")]
    pub since_seq: i64,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_since() -> i64 {
    -1
}
fn default_limit() -> i64 {
    1000
}

#[tauri::command]
pub fn one_shot_log(db: State<'_, Db>, args: LogArgs) -> Result<Vec<LogLine>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    list_log_lines_inner(&conn, &args.id, args.since_seq, args.limit).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn one_shot_kill(
    db: State<'_, Db>,
    state: State<'_, OneShotState>,
    id: String,
) -> Result<(), String> {
    let child_opt = {
        let mut map = state
            .children
            .lock()
            .map_err(|_| "state poisoned".to_string())?;
        map.remove(&id)
    };
    if let Some(child_arc) = child_opt {
        if let Ok(mut g) = child_arc.lock() {
            let _ = g.kill();
        }
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        let _ = append_log_line(&conn, &id, "stderr", "[killed by user]");
        finalize_run(&conn, &id, "killed", Some(-1), None).map_err(|e| e.to_string())?;
        Ok(())
    } else {
        // Already finished — turn this into a delete instead.
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM one_shot_log_lines WHERE run_id = ?1",
            params![id],
        )
        .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM one_shot_runs WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::run_migrations;

    fn fresh_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn
    }

    #[test]
    fn build_argv_defaults_to_print_stream_json_verbose() {
        let args = RunArgs {
            repo_name: "alpha".into(),
            cwd: "/tmp".into(),
            prompt: "hello".into(),
            ..Default::default()
        };
        let argv = build_argv(&args);
        assert_eq!(argv[0], "claude");
        assert_eq!(argv[1], "-p");
        assert!(argv.windows(2).any(|w| w == ["--output-format", "stream-json"]));
        assert!(argv.contains(&"--verbose".to_string()));
        // Prompt is the last element (positional).
        assert_eq!(argv.last().unwrap(), "hello");
    }

    #[test]
    fn build_argv_omits_prompt_when_empty() {
        let args = RunArgs {
            repo_name: "alpha".into(),
            cwd: "/tmp".into(),
            continue_last: true,
            ..Default::default()
        };
        let argv = build_argv(&args);
        assert!(argv.contains(&"--continue".to_string()));
        // No empty positional argument: the last token is a flag, not a blank
        // string, so claude doesn't try to interpret "" as a prompt.
        assert!(argv.last().unwrap() != "");
    }

    #[test]
    fn build_argv_threads_through_advanced_flags() {
        let args = RunArgs {
            repo_name: "alpha".into(),
            cwd: "/tmp".into(),
            prompt: "do it".into(),
            model: Some("sonnet".into()),
            permission_mode: Some("acceptEdits".into()),
            skip_permissions: true,
            effort: Some("high".into()),
            include_partial_messages: true,
            allowed_tools: vec!["Bash".into(), "Edit".into()],
            disallowed_tools: vec!["WebFetch".into()],
            add_dir: vec!["/tmp/extra".into(), "/tmp/extra2".into()],
            mcp_config: vec!["foo.json".into()],
            strict_mcp_config: true,
            max_budget_usd: Some(2.5),
            agent: Some("reviewer".into()),
            extra_args: vec!["--debug".into(), "api".into()],
            ..Default::default()
        };
        let argv = build_argv(&args);
        assert!(argv.windows(2).any(|w| w == ["--model", "sonnet"]));
        assert!(argv
            .windows(2)
            .any(|w| w == ["--permission-mode", "acceptEdits"]));
        assert!(argv.contains(&"--dangerously-skip-permissions".into()));
        assert!(argv.windows(2).any(|w| w == ["--effort", "high"]));
        assert!(argv.contains(&"--include-partial-messages".into()));
        assert!(argv.windows(2).any(|w| w == ["--allowedTools", "Bash,Edit"]));
        assert!(argv
            .windows(2)
            .any(|w| w == ["--disallowedTools", "WebFetch"]));
        assert!(argv.windows(2).any(|w| w == ["--add-dir", "/tmp/extra"]));
        assert!(argv.windows(2).any(|w| w == ["--add-dir", "/tmp/extra2"]));
        assert!(argv.windows(2).any(|w| w == ["--mcp-config", "foo.json"]));
        assert!(argv.contains(&"--strict-mcp-config".into()));
        assert!(argv.windows(2).any(|w| w == ["--max-budget-usd", "2.5"]));
        assert!(argv.windows(2).any(|w| w == ["--agent", "reviewer"]));
        // extra_args land after first-class flags but before the prompt.
        let dbg_idx = argv.iter().position(|a| a == "--debug").unwrap();
        let api_idx = argv.iter().position(|a| a == "api").unwrap();
        let prompt_idx = argv.iter().position(|a| a == "do it").unwrap();
        assert!(dbg_idx < prompt_idx);
        assert!(api_idx == dbg_idx + 1);
    }

    #[test]
    fn build_argv_skips_partial_when_format_is_text() {
        let args = RunArgs {
            repo_name: "alpha".into(),
            cwd: "/tmp".into(),
            prompt: "x".into(),
            output_format: Some("text".into()),
            include_partial_messages: true,
            ..Default::default()
        };
        let argv = build_argv(&args);
        // --include-partial-messages only works with stream-json, so we
        // strip it when the user picked text/json to avoid claude erroring.
        assert!(!argv.contains(&"--include-partial-messages".into()));
    }

    #[test]
    fn parse_total_cost_extracts_from_result_event() {
        let line = r#"{"type":"result","total_cost_usd":0.0123,"usage":{}}"#;
        assert_eq!(parse_total_cost(line), Some(0.0123));
        let other = r#"{"type":"assistant","message":{}}"#;
        assert_eq!(parse_total_cost(other), None);
        assert_eq!(parse_total_cost("not json"), None);
    }

    #[test]
    fn list_runs_filters_by_repo_and_status() {
        let conn = fresh_conn();
        let mk = |id: &str, repo_id: i64, status: &str| RunInfo {
            id: id.into(),
            repo_id,
            repo_name: format!("r{}", repo_id),
            cwd: "/tmp".into(),
            prompt: "p".into(),
            argv: vec!["claude".into()],
            status: status.into(),
            started_at: 1000 + repo_id,
            ended_at: None,
            exit_code: None,
            total_cost_usd: None,
            output_format: "stream-json".into(),
        };
        insert_run(&conn, &mk("a", 1, "running")).unwrap();
        insert_run(&conn, &mk("b", 1, "completed")).unwrap();
        insert_run(&conn, &mk("c", 2, "running")).unwrap();

        let by_repo = list_runs_inner(&conn, Some(1), None).unwrap();
        assert_eq!(by_repo.len(), 2);
        let by_status = list_runs_inner(&conn, None, Some("running")).unwrap();
        assert_eq!(by_status.len(), 2);
        let both = list_runs_inner(&conn, Some(1), Some("running")).unwrap();
        assert_eq!(both.len(), 1);
        assert_eq!(both[0].id, "a");
    }

    #[test]
    fn append_log_line_assigns_monotonic_seq_per_run() {
        let conn = fresh_conn();
        let info = RunInfo {
            id: "run1".into(),
            repo_id: 1,
            repo_name: "r1".into(),
            cwd: "/tmp".into(),
            prompt: "x".into(),
            argv: vec!["claude".into()],
            status: "running".into(),
            started_at: 1,
            ended_at: None,
            exit_code: None,
            total_cost_usd: None,
            output_format: "stream-json".into(),
        };
        insert_run(&conn, &info).unwrap();
        let s0 = append_log_line(&conn, "run1", "stdout", "first").unwrap();
        let s1 = append_log_line(&conn, "run1", "stdout", "second").unwrap();
        let s2 = append_log_line(&conn, "run1", "stderr", "err").unwrap();
        assert_eq!((s0, s1, s2), (0, 1, 2));

        let lines = list_log_lines_inner(&conn, "run1", -1, 100).unwrap();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].text, "first");
        assert_eq!(lines[2].stream, "stderr");

        // since_seq filter
        let after_first = list_log_lines_inner(&conn, "run1", 0, 100).unwrap();
        assert_eq!(after_first.len(), 2);
        assert_eq!(after_first[0].seq, 1);
    }

    #[test]
    fn finalize_run_sets_status_and_exit_code() {
        let conn = fresh_conn();
        let info = RunInfo {
            id: "x".into(),
            repo_id: 1,
            repo_name: "r".into(),
            cwd: "/".into(),
            prompt: "".into(),
            argv: vec!["claude".into()],
            status: "running".into(),
            started_at: 5,
            ended_at: None,
            exit_code: None,
            total_cost_usd: None,
            output_format: "stream-json".into(),
        };
        insert_run(&conn, &info).unwrap();
        finalize_run(&conn, "x", "completed", Some(0), Some(0.5)).unwrap();
        let row = get_run_inner(&conn, "x").unwrap().unwrap();
        assert_eq!(row.status, "completed");
        assert_eq!(row.exit_code, Some(0));
        assert_eq!(row.total_cost_usd, Some(0.5));
        assert!(row.ended_at.is_some());
    }
}

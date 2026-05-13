use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use tauri::State;

use crate::db::Db;

fn unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// One structured event extracted from a stream-json run.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunEvent {
    pub id: i64,
    pub run_id: String,
    pub seq: i64,
    pub ts: i64,
    pub event_type: String,
    pub tool_name: Option<String>,
    pub tool_use_id: Option<String>,
    pub input_json: Option<String>,
    pub output_json: Option<String>,
    pub thinking: Option<String>,
    pub is_error: bool,
}

/// Per-run summary row used by GET /graph/state.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    pub run_id: String,
    pub repo_name: String,
    pub status: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub total_cost_usd: Option<f64>,
    pub event_count: i64,
    pub tool_call_count: i64,
    /// Workflow role (e.g. "worker", "whitebox-validator") if this run was
    /// dispatched by the workflow engine; None for manual one-shots.
    pub agent_label: Option<String>,
    /// GitHub issue number that triggered this run via the workflow engine.
    pub issue_number: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphState {
    pub runs: Vec<RunSummary>,
}

// ---- Public API ---------------------------------------------------------

/// Return the structured decision events for a run.
///
/// Parses from `one_shot_log_lines` if `run_events` is still empty and the
/// run is finished — so historical runs work without any extra trigger.
pub fn get_run_events(conn: &Connection, run_id: &str) -> rusqlite::Result<Vec<RunEvent>> {
    ensure_parsed(conn, run_id)?;
    query_run_events(conn, run_id)
}

/// Snapshot of all runs enriched with event/tool-call counts and, when
/// dispatched by the workflow engine, the agent role from dispatch_log.
pub fn get_graph_state(conn: &Connection) -> rusqlite::Result<GraphState> {
    let mut stmt = conn.prepare(
        "SELECT r.id, r.repo_name, r.status, r.started_at, r.ended_at, r.total_cost_usd,
                COUNT(DISTINCT e.id) AS event_count,
                COUNT(DISTINCT CASE WHEN e.event_type = 'tool_use' THEN e.id END) AS tool_call_count,
                (SELECT d.directive_json  FROM dispatch_log d WHERE d.run_id = r.id LIMIT 1) AS directive_json,
                r.argv_json,
                (SELECT d.issue_number FROM dispatch_log d WHERE d.run_id = r.id LIMIT 1) AS issue_number
         FROM one_shot_runs r
         LEFT JOIN run_events e ON e.run_id = r.id
         GROUP BY r.id
         ORDER BY r.started_at DESC
         LIMIT 200",
    )?;
    let runs = stmt
        .query_map([], |row| {
            let directive_json: Option<String> = row.get(8)?;
            let argv_json: String = row.get(9)?;
            let issue_number: Option<i64> = row.get(10)?;

            let from_dispatch = directive_json.as_deref().and_then(|json| {
                serde_json::from_str::<Value>(json).ok()
                    .and_then(|v| v.get("role").and_then(|r| r.as_str()).map(String::from))
            });
            let agent_label = from_dispatch.or_else(|| role_from_argv(&argv_json));

            Ok(RunSummary {
                run_id: row.get(0)?,
                repo_name: row.get(1)?,
                status: row.get(2)?,
                started_at: row.get(3)?,
                ended_at: row.get(4)?,
                total_cost_usd: row.get(5)?,
                event_count: row.get(6)?,
                tool_call_count: row.get(7)?,
                agent_label,
                issue_number,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(GraphState { runs })
}

/// Parse `--name {role}-{mode}-issue{N}` out of a JSON argv array and return
/// the role component. Returns None for manual one-shots (no --name flag).
fn role_from_argv(argv_json: &str) -> Option<String> {
    let argv: Vec<String> = serde_json::from_str(argv_json).ok()?;
    let pos = argv.iter().position(|a| a == "--name")?;
    let name = argv.get(pos + 1)?;
    // Strip trailing -issue{digits}
    let without_issue = name.rsplit_once("-issue").and_then(|(prefix, suffix)| {
        if !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()) {
            Some(prefix)
        } else {
            None
        }
    })?;
    // Strip the mode (last hyphen-separated token, e.g. "-default")
    let role = without_issue
        .rsplit_once('-')
        .map(|(r, _)| r)
        .unwrap_or(without_issue);
    if role.is_empty() { None } else { Some(role.to_string()) }
}

/// Idempotent: parse and store run_events for a finished stream-json run.
/// No-op if already parsed or if the run is still running.
pub fn parse_and_store_run_events(conn: &Connection, run_id: &str) -> rusqlite::Result<()> {
    ensure_parsed(conn, run_id)
}

// ---- Internal -----------------------------------------------------------

fn ensure_parsed(conn: &Connection, run_id: &str) -> rusqlite::Result<()> {
    // Only process finished stream-json runs.
    let row: Option<(bool, String)> = conn
        .query_row(
            "SELECT status != 'running', output_format FROM one_shot_runs WHERE id = ?1",
            params![run_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let (is_done, fmt) = match row {
        Some(r) => r,
        None => return Ok(()),
    };
    if !is_done || fmt != "stream-json" {
        return Ok(());
    }

    // Already parsed?
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM run_events WHERE run_id = ?1",
        params![run_id],
        |row| row.get(0),
    )?;
    if count > 0 {
        return Ok(());
    }

    let lines = fetch_stdout_lines(conn, run_id)?;
    if lines.is_empty() {
        return Ok(());
    }
    let events = parse_stream_json(&lines);
    store_events(conn, run_id, &events)
}

fn query_run_events(conn: &Connection, run_id: &str) -> rusqlite::Result<Vec<RunEvent>> {
    let mut stmt = conn.prepare(
        "SELECT id, run_id, seq, ts, event_type, tool_name, tool_use_id,
                input_json, output_json, thinking, is_error
         FROM run_events WHERE run_id = ?1 ORDER BY seq ASC",
    )?;
    let rows = stmt.query_map(params![run_id], |row| {
        let is_error_i: i64 = row.get(10)?;
        Ok(RunEvent {
            id: row.get(0)?,
            run_id: row.get(1)?,
            seq: row.get(2)?,
            ts: row.get(3)?,
            event_type: row.get(4)?,
            tool_name: row.get(5)?,
            tool_use_id: row.get(6)?,
            input_json: row.get(7)?,
            output_json: row.get(8)?,
            thinking: row.get(9)?,
            is_error: is_error_i != 0,
        })
    })?
    .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Returns (text, ts_millis) for each stdout line, ordered by seq.
fn fetch_stdout_lines(conn: &Connection, run_id: &str) -> rusqlite::Result<Vec<(String, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT text, ts FROM one_shot_log_lines
         WHERE run_id = ?1 AND stream = 'stdout' ORDER BY seq ASC",
    )?;
    let rows = stmt
        .query_map(params![run_id], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

struct ParsedEvent {
    seq: i64,
    ts: i64,
    event_type: &'static str,
    tool_name: Option<String>,
    tool_use_id: Option<String>,
    input_json: Option<String>,
    output_json: Option<String>,
    thinking: Option<String>,
    is_error: bool,
}

/// Pure parser: converts stream-json NDJSON log lines into structured events.
///
/// Claude Code stream-json emits one JSON object per line:
///   {"type":"system","subtype":"init",...}
///   {"type":"assistant","message":{"content":[BLOCKS],...}}
///   {"type":"user","message":{"content":[TOOL_RESULT_BLOCKS],...}}
///   {"type":"result","subtype":"success","total_cost_usd":N,...}
///
/// Assistant content blocks: thinking | text | tool_use
/// User content blocks: tool_result (with tool_use_id linking back to tool_use)
fn parse_stream_json(lines: &[(String, i64)]) -> Vec<ParsedEvent> {
    let mut events: Vec<ParsedEvent> = vec![];
    let mut seq: i64 = 0;

    for (text, ts) in lines {
        let v: Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let event_type = match v.get("type").and_then(|t| t.as_str()) {
            Some(t) => t,
            None => continue,
        };

        match event_type {
            "assistant" => {
                let blocks = match v
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                {
                    Some(b) => b,
                    None => continue,
                };
                for block in blocks {
                    match block.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                        "thinking" => {
                            let text = block
                                .get("thinking")
                                .and_then(|t| t.as_str())
                                .unwrap_or("")
                                .to_string();
                            events.push(ParsedEvent {
                                seq,
                                ts: *ts,
                                event_type: "thinking",
                                tool_name: None,
                                tool_use_id: None,
                                input_json: None,
                                output_json: None,
                                thinking: Some(text),
                                is_error: false,
                            });
                            seq += 1;
                        }
                        "tool_use" => {
                            events.push(ParsedEvent {
                                seq,
                                ts: *ts,
                                event_type: "tool_use",
                                tool_name: block
                                    .get("name")
                                    .and_then(|n| n.as_str())
                                    .map(String::from),
                                tool_use_id: block
                                    .get("id")
                                    .and_then(|i| i.as_str())
                                    .map(String::from),
                                input_json: block.get("input").map(|i| i.to_string()),
                                output_json: None,
                                thinking: None,
                                is_error: false,
                            });
                            seq += 1;
                        }
                        _ => {}
                    }
                }
            }
            "user" => {
                let blocks = match v
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                {
                    Some(b) => b,
                    None => continue,
                };
                for block in blocks {
                    if block.get("type").and_then(|t| t.as_str()) != Some("tool_result") {
                        continue;
                    }
                    let is_error = block
                        .get("is_error")
                        .and_then(|e| e.as_bool())
                        .unwrap_or(false);
                    events.push(ParsedEvent {
                        seq,
                        ts: *ts,
                        event_type: "tool_result",
                        tool_name: None,
                        tool_use_id: block
                            .get("tool_use_id")
                            .and_then(|i| i.as_str())
                            .map(String::from),
                        input_json: None,
                        output_json: block.get("content").map(|c| c.to_string()),
                        thinking: None,
                        is_error,
                    });
                    seq += 1;
                }
            }
            "result" => {
                let is_error = v
                    .get("subtype")
                    .and_then(|s| s.as_str())
                    .map(|s| s != "success")
                    .unwrap_or(false);
                events.push(ParsedEvent {
                    seq,
                    ts: *ts,
                    event_type: "result",
                    tool_name: None,
                    tool_use_id: None,
                    input_json: None,
                    output_json: Some(v.to_string()),
                    thinking: None,
                    is_error,
                });
                seq += 1;
            }
            _ => {}
        }
    }
    events
}

fn store_events(conn: &Connection, run_id: &str, events: &[ParsedEvent]) -> rusqlite::Result<()> {
    for ev in events {
        conn.execute(
            "INSERT OR IGNORE INTO run_events
             (run_id, seq, ts, event_type, tool_name, tool_use_id,
              input_json, output_json, thinking, is_error)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                run_id,
                ev.seq,
                ev.ts,
                ev.event_type,
                ev.tool_name,
                ev.tool_use_id,
                ev.input_json,
                ev.output_json,
                ev.thinking,
                ev.is_error as i64,
            ],
        )?;
    }
    Ok(())
}

// ---- Dispatch log -------------------------------------------------------

/// One row from dispatch_log — records why an issue was (or wasn't) dispatched.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DispatchEntry {
    pub id: i64,
    pub issue_number: i64,
    pub repo_full_name: String,
    pub matched_at: i64,
    /// 0-based index of the matched dispatch rule; None = no rule matched.
    pub rule_index: Option<i64>,
    /// "spawn_fresh" | "no_action" | "wait" | "human_review"
    pub directive_type: String,
    pub directive_json: String,
    pub run_id: Option<String>,
}

/// Write one dispatch decision to the log. Returns the new row id so the
/// caller can back-fill `run_id` once the spawn completes.
pub fn write_dispatch_log(
    conn: &Connection,
    issue_number: i64,
    repo_full_name: &str,
    rule_index: Option<usize>,
    directive_type: &str,
    directive_json: &str,
) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO dispatch_log
         (issue_number, repo_full_name, matched_at, rule_index, directive_type, directive_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            issue_number,
            repo_full_name,
            unix_secs(),
            rule_index.map(|i| i as i64),
            directive_type,
            directive_json,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Back-fill the run_id on a dispatch_log row after the spawn completes.
pub fn update_dispatch_run_id(
    conn: &Connection,
    dispatch_id: i64,
    run_id: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE dispatch_log SET run_id = ?1 WHERE id = ?2",
        params![run_id, dispatch_id],
    )?;
    Ok(())
}

pub fn get_dispatch_log(
    conn: &Connection,
    repo_full_name: &str,
    issue_number: i64,
) -> rusqlite::Result<Vec<DispatchEntry>> {
    let mut stmt = conn.prepare(
        "SELECT id, issue_number, repo_full_name, matched_at, rule_index,
                directive_type, directive_json, run_id
         FROM dispatch_log
         WHERE repo_full_name = ?1 AND issue_number = ?2
         ORDER BY matched_at DESC",
    )?;
    let rows = stmt
        .query_map(params![repo_full_name, issue_number], |row| {
            Ok(DispatchEntry {
                id: row.get(0)?,
                issue_number: row.get(1)?,
                repo_full_name: row.get(2)?,
                matched_at: row.get(3)?,
                rule_index: row.get(4)?,
                directive_type: row.get(5)?,
                directive_json: row.get(6)?,
                run_id: row.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

// ---- Graph edges --------------------------------------------------------

/// Insert a directed edge between two graph nodes. Idempotent — skips the
/// insert if (from_id, edge_type, to_id) already exists.
///
/// from/to types: "issue" | "run" | "event"
/// edge types:   "dispatched_to" | "retried_as" | "blocked_by"
///
/// Issue from_id format: "{repo_full_name}#{issue_number}"  e.g. "org/repo#42"
pub fn write_graph_edge(
    conn: &Connection,
    from_type: &str,
    from_id: &str,
    edge_type: &str,
    to_type: &str,
    to_id: &str,
    meta_json: Option<&str>,
) -> rusqlite::Result<()> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM graph_edges WHERE from_id=?1 AND edge_type=?2 AND to_id=?3)",
        params![from_id, edge_type, to_id],
        |row| row.get(0),
    )?;
    if exists {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO graph_edges
         (from_type, from_id, edge_type, to_type, to_id, meta_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![from_type, from_id, edge_type, to_type, to_id, meta_json, unix_secs()],
    )?;
    Ok(())
}

// ---- Issue deps / blocking ----------------------------------------------

/// Parse `<!-- deps: #1 #2 #3 -->` from an issue body.
/// Returns issue numbers within the same repo.
fn parse_deps(body: &str) -> Vec<u64> {
    let needle = "<!-- deps:";
    let Some(start) = body.find(needle) else {
        return vec![];
    };
    let after = &body[start + needle.len()..];
    let Some(end) = after.find("-->") else {
        return vec![];
    };
    after[..end]
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter_map(|tok| tok.trim().trim_start_matches('#').parse::<u64>().ok())
        .collect()
}

/// For a given issue, write `blocked_by` edges for every number listed in
/// its `<!-- deps: -->` marker. Idempotent — safe to call on every dispatch.
pub fn sync_issue_deps(
    conn: &Connection,
    issue_number: u64,
    repo_full_name: &str,
    body: &str,
) -> rusqlite::Result<()> {
    let from_id = format!("{repo_full_name}#{issue_number}");
    for dep in parse_deps(body) {
        let to_id = format!("{repo_full_name}#{dep}");
        write_graph_edge(conn, "issue", &from_id, "blocked_by", "issue", &to_id, None)?;
    }
    Ok(())
}

/// One blocked issue with the list of issues blocking it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockingItem {
    pub issue_id: String,
    pub title: Option<String>,
    pub blocked_by: Vec<BlockerRef>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockerRef {
    pub issue_id: String,
    pub title: Option<String>,
}

/// Return all `blocked_by` edges, enriched with issue titles from cache.
pub fn get_blocking_graph(conn: &Connection) -> rusqlite::Result<Vec<BlockingItem>> {
    // Fetch all blocked_by edges.
    let mut stmt = conn.prepare(
        "SELECT from_id, to_id FROM graph_edges WHERE edge_type = 'blocked_by' ORDER BY from_id, to_id",
    )?;
    let pairs: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    // Group by from_id.
    let mut grouped: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for (from, to) in &pairs {
        grouped.entry(from.clone()).or_default().push(to.clone());
    }

    // Collect unique issue ids to look up titles in one query.
    let all_ids: std::collections::HashSet<String> = pairs
        .iter()
        .flat_map(|(f, t)| [f.clone(), t.clone()])
        .collect();
    let titles = fetch_issue_titles(conn, &all_ids)?;

    Ok(grouped
        .into_iter()
        .map(|(from_id, blockers)| BlockingItem {
            title: titles.get(&from_id).cloned(),
            blocked_by: blockers
                .into_iter()
                .map(|to_id| BlockerRef {
                    title: titles.get(&to_id).cloned(),
                    issue_id: to_id,
                })
                .collect(),
            issue_id: from_id,
        })
        .collect())
}

/// Look up issue titles from the cached `issues` table.
/// `ids` use the format "repo_full_name#number".
fn fetch_issue_titles(
    conn: &Connection,
    ids: &std::collections::HashSet<String>,
) -> rusqlite::Result<std::collections::HashMap<String, String>> {
    let mut map = std::collections::HashMap::new();
    for id in ids {
        let Some((repo, num_str)) = id.split_once('#') else {
            continue;
        };
        let Ok(num) = num_str.parse::<i64>() else {
            continue;
        };
        let title: Option<String> = conn
            .query_row(
                "SELECT title FROM issues WHERE repo_full_name = ?1 AND number = ?2",
                params![repo, num],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(t) = title {
            map.insert(id.clone(), t);
        }
    }
    Ok(map)
}

// ---- Issue trace --------------------------------------------------------

/// Full causal trace for one issue: every dispatch decision + the linked run
/// (if any) + that run's structured decision events.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueTrace {
    pub issue_number: i64,
    pub repo_full_name: String,
    pub dispatches: Vec<DispatchWithRun>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DispatchWithRun {
    pub dispatch: DispatchEntry,
    pub run: Option<crate::one_shot::RunInfo>,
    pub decisions: Vec<RunEvent>,
}

pub fn get_issue_trace(
    conn: &Connection,
    repo_full_name: &str,
    issue_number: i64,
) -> rusqlite::Result<IssueTrace> {
    let dispatches = get_dispatch_log(conn, repo_full_name, issue_number)?;
    let mut result = vec![];
    for d in dispatches {
        let run = d
            .run_id
            .as_deref()
            .map(|id| crate::one_shot::get_run_inner(conn, id))
            .transpose()?
            .flatten();
        let decisions = match d.run_id.as_deref() {
            Some(id) => get_run_events(conn, id)?,
            None => vec![],
        };
        result.push(DispatchWithRun {
            dispatch: d,
            run,
            decisions,
        });
    }
    Ok(IssueTrace {
        issue_number,
        repo_full_name: repo_full_name.to_string(),
        dispatches: result,
    })
}

// ---- Internal helpers ---------------------------------------------------

pub fn get_dispatch_log_recent(
    conn: &Connection,
    repo_full_name: &str,
    limit: i64,
) -> rusqlite::Result<Vec<DispatchEntry>> {
    let mut stmt = conn.prepare(
        "SELECT id, issue_number, repo_full_name, matched_at, rule_index,
                directive_type, directive_json, run_id
         FROM dispatch_log
         WHERE repo_full_name = ?1
         ORDER BY matched_at DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![repo_full_name, limit], |row| {
        Ok(DispatchEntry {
            id: row.get(0)?,
            issue_number: row.get(1)?,
            repo_full_name: row.get(2)?,
            matched_at: row.get(3)?,
            rule_index: row.get(4)?,
            directive_type: row.get(5)?,
            directive_json: row.get(6)?,
            run_id: row.get(7)?,
        })
    })?
    .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

// ---- Tauri commands -----------------------------------------------------

#[tauri::command]
pub fn dispatch_log_recent_cmd(
    repo_full_name: String,
    limit: Option<i64>,
    db: State<'_, Db>,
) -> Result<Vec<DispatchEntry>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    get_dispatch_log_recent(&conn, &repo_full_name, limit.unwrap_or(40))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn graph_state_cmd(db: State<'_, Db>) -> Result<GraphState, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    get_graph_state(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn graph_blocking_cmd(db: State<'_, Db>) -> Result<Vec<BlockingItem>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    get_blocking_graph(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn graph_run_events_cmd(run_id: String, db: State<'_, Db>) -> Result<Vec<RunEvent>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    get_run_events(&conn, &run_id).map_err(|e| e.to_string())
}

// ---- Tests --------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (rusqlite::Connection, String) {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::db::run_migrations(&conn).unwrap();
        // Insert a minimal completed stream-json run.
        conn.execute(
            "INSERT INTO repos (id,name,full_name,html_url,private,default_branch,stargazers_count,updated_at,synced_at)
             VALUES (1,'r','o/r','http://x',0,'main',0,'now','now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO one_shot_runs (id,repo_id,repo_name,cwd,argv_json,prompt,status,started_at,output_format)
             VALUES ('run-1',1,'r','/','[]','hi','completed',1,'stream-json')",
            [],
        )
        .unwrap();
        (conn, "run-1".to_string())
    }

    fn insert_log(conn: &rusqlite::Connection, run_id: &str, seq: i64, text: &str) {
        conn.execute(
            "INSERT INTO one_shot_log_lines (run_id,seq,ts,stream,text) VALUES (?1,?2,1000,'stdout',?3)",
            params![run_id, seq, text],
        )
        .unwrap();
    }

    #[test]
    fn parses_tool_use_and_result() {
        let (conn, run_id) = setup();
        let assistant = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"ls"}}]}}"#;
        let user = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1","content":[{"type":"text","text":"file.txt"}],"is_error":false}]}}"#;
        let result = r#"{"type":"result","subtype":"success","total_cost_usd":0.01}"#;
        insert_log(&conn, &run_id, 0, assistant);
        insert_log(&conn, &run_id, 1, user);
        insert_log(&conn, &run_id, 2, result);

        let events = get_run_events(&conn, &run_id).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].event_type, "tool_use");
        assert_eq!(events[0].tool_name.as_deref(), Some("Bash"));
        assert_eq!(events[0].tool_use_id.as_deref(), Some("toolu_1"));
        assert!(events[0].input_json.as_deref().unwrap().contains("ls"));

        assert_eq!(events[1].event_type, "tool_result");
        assert_eq!(events[1].tool_use_id.as_deref(), Some("toolu_1"));
        assert!(!events[1].is_error);

        assert_eq!(events[2].event_type, "result");
        assert!(!events[2].is_error);
    }

    #[test]
    fn parses_thinking_block() {
        let (conn, run_id) = setup();
        let msg = r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"I should check..."}]}}"#;
        insert_log(&conn, &run_id, 0, msg);

        let events = get_run_events(&conn, &run_id).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "thinking");
        assert_eq!(events[0].thinking.as_deref(), Some("I should check..."));
    }

    #[test]
    fn idempotent_second_call() {
        let (conn, run_id) = setup();
        let msg = r#"{"type":"result","subtype":"success","total_cost_usd":0.01}"#;
        insert_log(&conn, &run_id, 0, msg);

        get_run_events(&conn, &run_id).unwrap();
        get_run_events(&conn, &run_id).unwrap(); // must not double-insert

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM run_events WHERE run_id = 'run-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn skips_running_run() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::db::run_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO repos (id,name,full_name,html_url,private,default_branch,stargazers_count,updated_at,synced_at)
             VALUES (1,'r','o/r','http://x',0,'main',0,'now','now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO one_shot_runs (id,repo_id,repo_name,cwd,argv_json,prompt,status,started_at,output_format)
             VALUES ('run-live',1,'r','/','[]','hi','running',1,'stream-json')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO one_shot_log_lines (run_id,seq,ts,stream,text) VALUES ('run-live',0,1000,'stdout','{\"type\":\"result\"}')",
            [],
        )
        .unwrap();

        let events = get_run_events(&conn, "run-live").unwrap();
        assert!(events.is_empty(), "should not parse a still-running run");
    }

    #[test]
    fn graph_state_includes_tool_counts() {
        let (conn, run_id) = setup();
        let assistant = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"ls"}}]}}"#;
        insert_log(&conn, &run_id, 0, assistant);
        get_run_events(&conn, &run_id).unwrap(); // trigger parse

        let state = get_graph_state(&conn).unwrap();
        let summary = state.runs.iter().find(|r| r.run_id == run_id).unwrap();
        assert_eq!(summary.tool_call_count, 1);
        assert_eq!(summary.event_count, 1);
    }

    fn base_conn() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::db::run_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO repos (id,name,full_name,html_url,private,default_branch,stargazers_count,updated_at,synced_at)
             VALUES (1,'r','org/r','http://x',0,'main',0,'now','now')",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn sync_issue_deps_writes_blocked_by_edges() {
        let conn = base_conn();
        let body = "Some text\n<!-- deps: #1 #2 -->\nmore text";
        sync_issue_deps(&conn, 42, "org/r", body).unwrap();

        let blocking = get_blocking_graph(&conn).unwrap();
        assert_eq!(blocking.len(), 1);
        assert_eq!(blocking[0].issue_id, "org/r#42");
        let blockers: Vec<&str> = blocking[0].blocked_by.iter().map(|b| b.issue_id.as_str()).collect();
        assert!(blockers.contains(&"org/r#1"));
        assert!(blockers.contains(&"org/r#2"));
    }

    #[test]
    fn sync_issue_deps_is_idempotent() {
        let conn = base_conn();
        let body = "<!-- deps: #5 -->";
        sync_issue_deps(&conn, 10, "org/r", body).unwrap();
        sync_issue_deps(&conn, 10, "org/r", body).unwrap(); // second call must not double-insert

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM graph_edges WHERE edge_type='blocked_by'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn sync_issue_deps_no_marker_writes_nothing() {
        let conn = base_conn();
        sync_issue_deps(&conn, 7, "org/r", "no marker here").unwrap();
        let blocking = get_blocking_graph(&conn).unwrap();
        assert!(blocking.is_empty());
    }

    #[test]
    fn dispatch_log_round_trips() {
        let conn = base_conn();
        conn.execute(
            "INSERT INTO one_shot_runs (id,repo_id,repo_name,cwd,argv_json,prompt,status,started_at,output_format)
             VALUES ('run-x',1,'r','/','[]','p','completed',1,'stream-json')",
            [],
        ).unwrap();

        let id = write_dispatch_log(&conn, 99, "org/r", Some(0), "spawn_fresh", r#"{"directive":"spawn_fresh","role":"implementer"}"#).unwrap();
        update_dispatch_run_id(&conn, id, "run-x").unwrap();

        let entries = get_dispatch_log(&conn, "org/r", 99).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].rule_index, Some(0));
        assert_eq!(entries[0].directive_type, "spawn_fresh");
        assert_eq!(entries[0].run_id.as_deref(), Some("run-x"));
    }

    #[test]
    fn blocking_graph_enriches_titles_from_cache() {
        let conn = base_conn();
        // Insert two issues into cache.
        conn.execute(
            "INSERT INTO issues (id,repo_full_name,number,title,html_url,labels_json,synced_at)
             VALUES (42,'org/r',42,'Feature X','http://x','[]','now')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO issues (id,repo_full_name,number,title,html_url,labels_json,synced_at)
             VALUES (1,'org/r',1,'Design doc','http://x','[]','now')",
            [],
        ).unwrap();

        sync_issue_deps(&conn, 42, "org/r", "<!-- deps: #1 -->").unwrap();
        let blocking = get_blocking_graph(&conn).unwrap();

        assert_eq!(blocking[0].title.as_deref(), Some("Feature X"));
        assert_eq!(blocking[0].blocked_by[0].title.as_deref(), Some("Design doc"));
    }
}

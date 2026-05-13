use crate::cache;
use crate::db::Db;
use crate::github::{Issue, PullRequest, Repo};
use crate::local_repo::{inspect_at, LocalRepoInspection};
use crate::one_shot::{
    self, get_run_inner, list_log_lines_inner, list_runs_inner, LogLine, OneShotState, RunArgs,
    RunInfo,
};
use crate::settings::{get_settings_inner, save_workflow_path_inner};
use crate::terminal::{Registry, SessionInfo};
use crate::workflow::WorkflowStatus;
use axum::{
    extract::{Path, Query, State},
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::Path as StdPath;
use std::sync::{Arc, RwLock};
use tauri::AppHandle;

#[derive(Clone)]
pub struct ServerCtx {
    pub db: Db,
    pub registry: Registry,
    pub one_shot: OneShotState,
    /// `None` when the server is constructed for tests where no Tauri runtime
    /// exists. `POST /one-shot` returns 503 in that case; read-only routes
    /// keep working against the shared SQLite Db.
    pub app_handle: Option<AppHandle>,
    pub token: String,
    /// Shared workflow status — written by lib.rs after startup, read by
    /// `GET /workflow`. `None` only in test contexts that don't exercise the
    /// workflow endpoints.
    pub workflow_status: Option<Arc<RwLock<WorkflowStatus>>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RepoDetail {
    repo: Repo,
    issues: Vec<Issue>,
    prs: Vec<PullRequest>,
    local: LocalRepoInspection,
}

pub fn router(ctx: ServerCtx) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/repos", get(list_repos))
        .route("/repos/{name}", get(get_repo_detail))
        .route("/sessions", get(list_sessions))
        .route("/one-shot", get(list_one_shot).post(start_one_shot))
        .route("/one-shot/{id}", get(get_one_shot).delete(delete_one_shot))
        .route("/one-shot/{id}/log", get(get_one_shot_log))
        .route("/workflow", get(get_workflow_status))
        .route("/workflow/path", post(set_workflow_path))
        .route("/graph/state", get(graph_state))
        .route("/graph/runs/{id}/decisions", get(graph_run_decisions))
        .route("/graph/issues/{n}/why", get(graph_issue_why))
        .route("/graph/issues/{n}/trace", get(graph_issue_trace))
        .route("/graph/blocking", get(graph_blocking))
        .with_state(ctx)
}

pub async fn bind_and_serve(ctx: ServerCtx, app_dir: &StdPath) -> std::io::Result<()> {
    let app = router(ctx.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    write_meta_file(app_dir, addr, &ctx.token)?;
    eprintln!(
        "agent-sweet-home: HTTP API listening on http://{} (token in {})",
        addr,
        app_dir.join("server.json").display()
    );
    axum::serve(listener, app).await?;
    Ok(())
}

fn write_meta_file(app_dir: &StdPath, addr: SocketAddr, token: &str) -> std::io::Result<()> {
    let meta = serde_json::json!({
        "host": addr.ip().to_string(),
        "port": addr.port(),
        "token": token,
    });
    let path = app_dir.join("server.json");
    std::fs::write(&path, serde_json::to_string_pretty(&meta).unwrap_or_default())?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }

    Ok(())
}

// ===== Handlers =====

async fn health() -> &'static str {
    "ok"
}

async fn list_repos(
    State(ctx): State<ServerCtx>,
    headers: HeaderMap,
) -> Result<Json<Vec<Repo>>, StatusCode> {
    auth(&ctx, &headers)?;
    let conn = ctx
        .db
        .0
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let repos = cache::list_repos(&conn).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(repos))
}

async fn get_repo_detail(
    State(ctx): State<ServerCtx>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Result<Json<RepoDetail>, StatusCode> {
    auth(&ctx, &headers)?;
    let (repo, issues, prs, base_path) = {
        let conn = ctx
            .db
            .0
            .lock()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let repo = cache::get_repo_by_name(&conn, &name)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        let issues = cache::list_issues(&conn, &repo.full_name)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let prs = cache::list_prs(&conn, &repo.full_name)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let base_path = get_settings_inner(&conn)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .local_base_path;
        (repo, issues, prs, base_path)
    };
    let local = inspect_at(&base_path, &repo.name);
    Ok(Json(RepoDetail {
        repo,
        issues,
        prs,
        local,
    }))
}

fn auth(ctx: &ServerCtx, headers: &HeaderMap) -> Result<(), StatusCode> {
    let want = format!("Bearer {}", ctx.token);
    match headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        Some(got) if got == want => Ok(()),
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

#[derive(Deserialize)]
struct SessionsQuery {
    repo: Option<String>,
    #[serde(rename = "repoId")]
    repo_id: Option<i64>,
}

async fn list_sessions(
    State(ctx): State<ServerCtx>,
    Query(q): Query<SessionsQuery>,
    headers: HeaderMap,
) -> Result<Json<Vec<SessionInfo>>, StatusCode> {
    auth(&ctx, &headers)?;
    let mut sessions = ctx.registry.snapshot_all_public(q.repo_id);
    if let Some(name) = q.repo {
        sessions.retain(|s| s.repo_name == name);
    }
    Ok(Json(sessions))
}

#[derive(Deserialize)]
struct OneShotListQuery {
    repo: Option<String>,
    #[serde(rename = "repoId")]
    repo_id: Option<i64>,
    status: Option<String>,
}

async fn list_one_shot(
    State(ctx): State<ServerCtx>,
    Query(q): Query<OneShotListQuery>,
    headers: HeaderMap,
) -> Result<Json<Vec<RunInfo>>, StatusCode> {
    auth(&ctx, &headers)?;
    let conn = ctx
        .db
        .0
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut runs = list_runs_inner(&conn, q.repo_id, q.status.as_deref())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if let Some(name) = q.repo {
        runs.retain(|r| r.repo_name == name);
    }
    Ok(Json(runs))
}

async fn start_one_shot(
    State(ctx): State<ServerCtx>,
    headers: HeaderMap,
    Json(args): Json<RunArgs>,
) -> Result<Json<RunInfo>, StatusCode> {
    auth(&ctx, &headers)?;
    let app = ctx.app_handle.clone().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    one_shot::start_run(&app, &ctx.db, &ctx.one_shot, args)
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

async fn get_one_shot(
    State(ctx): State<ServerCtx>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<RunInfo>, StatusCode> {
    auth(&ctx, &headers)?;
    let conn = ctx
        .db
        .0
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let run = get_run_inner(&conn, &id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(run))
}

#[derive(Deserialize, Default)]
struct LogQuery {
    #[serde(default = "default_since")]
    since: i64,
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_since() -> i64 {
    -1
}

fn default_limit() -> i64 {
    1000
}

async fn get_one_shot_log(
    State(ctx): State<ServerCtx>,
    Path(id): Path<String>,
    Query(q): Query<LogQuery>,
    headers: HeaderMap,
) -> Result<Json<Vec<LogLine>>, StatusCode> {
    auth(&ctx, &headers)?;
    let conn = ctx
        .db
        .0
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let lines = list_log_lines_inner(&conn, &id, q.since, q.limit)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(lines))
}

/// `GET /workflow` — returns the current workflow status.
#[derive(Serialize)]
struct WorkflowStatusResp {
    path: String,
    exists: bool,
    loaded: bool,
    error: Option<String>,
}

async fn get_workflow_status(
    State(ctx): State<ServerCtx>,
    headers: HeaderMap,
) -> Result<Json<WorkflowStatusResp>, StatusCode> {
    auth(&ctx, &headers)?;
    let wf = ctx
        .workflow_status
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let s = wf.read().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(WorkflowStatusResp {
        path: s.path.clone(),
        exists: s.exists,
        loaded: s.loaded,
        error: s.error.clone(),
    }))
}

/// `POST /workflow/path` — persists a new workflow_path to the settings table.
/// The workflow engine does NOT hot-reload; a full app restart is required
/// for the new path to take effect.
#[derive(Deserialize)]
struct SetWorkflowPathBody {
    path: String,
}

#[derive(Serialize)]
struct SetWorkflowPathResp {
    ok: bool,
    path: String,
    note: &'static str,
}

async fn set_workflow_path(
    State(ctx): State<ServerCtx>,
    headers: HeaderMap,
    Json(body): Json<SetWorkflowPathBody>,
) -> Result<Json<SetWorkflowPathResp>, StatusCode> {
    auth(&ctx, &headers)?;
    let conn = ctx
        .db
        .0
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    save_workflow_path_inner(&conn, &body.path)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(SetWorkflowPathResp {
        ok: true,
        path: body.path,
        note: "Restart the app for the new workflow path to take effect.",
    }))
}

async fn delete_one_shot(
    State(ctx): State<ServerCtx>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, StatusCode> {
    auth(&ctx, &headers)?;
    // Same semantics as the Tauri command: kill if running, otherwise delete.
    if ctx.one_shot.is_running(&id) {
        if let Some(child_arc) = ctx.one_shot.take_child(&id) {
            if let Ok(mut g) = child_arc.lock() {
                let _ = g.kill();
            }
        }
        return Ok(StatusCode::ACCEPTED);
    }
    let conn = ctx
        .db
        .0
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    conn.execute(
        "DELETE FROM one_shot_log_lines WHERE run_id = ?1",
        rusqlite::params![&id],
    )
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let n = conn
        .execute(
            "DELETE FROM one_shot_runs WHERE id = ?1",
            rusqlite::params![&id],
        )
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if n == 0 {
        Err(StatusCode::NOT_FOUND)
    } else {
        Ok(StatusCode::NO_CONTENT)
    }
}

#[derive(Deserialize)]
struct IssueGraphQuery {
    repo: String,
}

async fn graph_blocking(
    State(ctx): State<ServerCtx>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::graph::BlockingItem>>, StatusCode> {
    auth(&ctx, &headers)?;
    let conn = ctx
        .db
        .0
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    crate::graph::get_blocking_graph(&conn)
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn graph_issue_why(
    State(ctx): State<ServerCtx>,
    Path(n): Path<i64>,
    Query(q): Query<IssueGraphQuery>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::graph::DispatchEntry>>, StatusCode> {
    auth(&ctx, &headers)?;
    let conn = ctx
        .db
        .0
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    crate::graph::get_dispatch_log(&conn, &q.repo, n)
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn graph_issue_trace(
    State(ctx): State<ServerCtx>,
    Path(n): Path<i64>,
    Query(q): Query<IssueGraphQuery>,
    headers: HeaderMap,
) -> Result<Json<crate::graph::IssueTrace>, StatusCode> {
    auth(&ctx, &headers)?;
    let conn = ctx
        .db
        .0
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    crate::graph::get_issue_trace(&conn, &q.repo, n)
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn graph_state(
    State(ctx): State<ServerCtx>,
    headers: HeaderMap,
) -> Result<Json<crate::graph::GraphState>, StatusCode> {
    auth(&ctx, &headers)?;
    let conn = ctx
        .db
        .0
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    crate::graph::get_graph_state(&conn).map(Json).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn graph_run_decisions(
    State(ctx): State<ServerCtx>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::graph::RunEvent>>, StatusCode> {
    auth(&ctx, &headers)?;
    let conn = ctx
        .db
        .0
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    crate::graph::get_run_events(&conn, &id)
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache;
    use crate::github::{Label, Repo};
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use rusqlite::Connection;
    use tower::ServiceExt;

    const TOKEN: &str = "test-token-abc";

    fn ctx_with_repo() -> ServerCtx {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::run_migrations(&conn).unwrap();
        let repo = Repo {
            id: 1,
            name: "alpha".into(),
            full_name: "octocat/alpha".into(),
            description: Some("desc".into()),
            html_url: "https://github.com/octocat/alpha".into(),
            private: false,
            default_branch: "main".into(),
            stargazers_count: 5,
            language: Some("Rust".into()),
            updated_at: "2026-01-01T00:00:00Z".into(),
        };
        cache::replace_repos(&conn, &[repo]).unwrap();
        cache::replace_issues(
            &conn,
            "octocat/alpha",
            &[crate::github::Issue {
                id: 100,
                number: 42,
                title: "fix".into(),
                html_url: "".into(),
                labels: vec![Label {
                    name: "bug".into(),
                    color: "ff0000".into(),
                }],
            }],
        )
        .unwrap();

        ServerCtx {
            db: Db::from_connection(conn),
            registry: crate::terminal::Registry::new(),
            one_shot: OneShotState::new(),
            app_handle: None,
            token: TOKEN.into(),
            workflow_status: None,
        }
    }

    #[tokio::test]
    async fn health_does_not_require_auth() {
        let app = router(ctx_with_repo());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn list_repos_rejects_missing_token() {
        let app = router(ctx_with_repo());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/repos")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn list_repos_rejects_wrong_token() {
        let app = router(ctx_with_repo());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/repos")
                    .header(AUTHORIZATION, "Bearer wrong")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn list_repos_returns_cached_repos() {
        let app = router(ctx_with_repo());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/repos")
                    .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), 1024 * 64).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 1);
        assert_eq!(v[0]["name"], "alpha");
    }

    #[tokio::test]
    async fn get_repo_detail_returns_repo_with_issues_and_local() {
        let app = router(ctx_with_repo());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/repos/alpha")
                    .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), 1024 * 64).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["repo"]["name"], "alpha");
        assert_eq!(v["issues"].as_array().unwrap().len(), 1);
        assert_eq!(v["issues"][0]["number"], 42);
        assert!(v["local"].is_object());
    }

    #[tokio::test]
    async fn get_repo_detail_returns_404_for_unknown_name() {
        let app = router(ctx_with_repo());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/repos/missing")
                    .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn list_sessions_requires_auth_and_returns_empty_when_no_terminals() {
        let ctx = ctx_with_repo();
        let app = router(ctx);

        let unauth = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/sessions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauth.status(), StatusCode::UNAUTHORIZED);

        let ok = app
            .oneshot(
                Request::builder()
                    .uri("/sessions")
                    .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
        let body = to_bytes(ok.into_body(), 1024 * 64).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 0);
    }

    fn seed_one_shot_run(ctx: &ServerCtx, id: &str, repo_id: i64, status: &str) {
        let conn = ctx.db.0.lock().unwrap();
        conn.execute(
            "INSERT INTO one_shot_runs
             (id, repo_id, repo_name, cwd, argv_json, prompt, status, started_at, output_format)
             VALUES (?1, ?2, 'alpha', '/tmp', '[\"claude\"]', 'p', ?3, 100, 'stream-json')",
            rusqlite::params![id, repo_id, status],
        )
        .unwrap();
    }

    fn seed_one_shot_log(ctx: &ServerCtx, run_id: &str, seq: i64, text: &str) {
        let conn = ctx.db.0.lock().unwrap();
        conn.execute(
            "INSERT INTO one_shot_log_lines (run_id, seq, ts, stream, text)
             VALUES (?1, ?2, 1, 'stdout', ?3)",
            rusqlite::params![run_id, seq, text],
        )
        .unwrap();
    }

    #[tokio::test]
    async fn list_one_shot_returns_runs_with_filters() {
        let ctx = ctx_with_repo();
        seed_one_shot_run(&ctx, "alpha-1-aa", 1, "running");
        seed_one_shot_run(&ctx, "alpha-2-bb", 1, "completed");
        seed_one_shot_run(&ctx, "beta-3-cc", 2, "running");
        let app = router(ctx);

        let all = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/one-shot")
                    .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(all.into_body(), 1024 * 64).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 3);

        let by_status = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/one-shot?status=running&repoId=1")
                    .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(by_status.into_body(), 1024 * 64).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 1);
        assert_eq!(v[0]["id"], "alpha-1-aa");
    }

    #[tokio::test]
    async fn get_one_shot_returns_404_for_unknown_id() {
        let app = router(ctx_with_repo());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/one-shot/missing")
                    .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn get_one_shot_log_returns_lines_filtered_by_since() {
        let ctx = ctx_with_repo();
        seed_one_shot_run(&ctx, "alpha-9-zz", 1, "completed");
        seed_one_shot_log(&ctx, "alpha-9-zz", 0, "first");
        seed_one_shot_log(&ctx, "alpha-9-zz", 1, "second");
        seed_one_shot_log(&ctx, "alpha-9-zz", 2, "third");
        let app = router(ctx);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/one-shot/alpha-9-zz/log?since=0")
                    .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(resp.into_body(), 1024 * 64).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 2);
        assert_eq!(v[0]["text"], "second");
        assert_eq!(v[1]["text"], "third");
    }

    #[tokio::test]
    async fn delete_one_shot_removes_finished_run_and_logs() {
        let ctx = ctx_with_repo();
        seed_one_shot_run(&ctx, "alpha-9-zz", 1, "completed");
        seed_one_shot_log(&ctx, "alpha-9-zz", 0, "x");
        let db_for_check = ctx.db.clone();
        let app = router(ctx);

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/one-shot/alpha-9-zz")
                    .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        let conn = db_for_check.0.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM one_shot_runs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
        let lines: i64 = conn
            .query_row("SELECT COUNT(*) FROM one_shot_log_lines", [], |r| r.get(0))
            .unwrap();
        assert_eq!(lines, 0);
    }

    #[tokio::test]
    async fn post_one_shot_returns_503_when_no_app_handle() {
        let app = router(ctx_with_repo());
        let body = serde_json::json!({
            "repoId": 1,
            "repoName": "alpha",
            "cwd": "/tmp",
            "prompt": "hi"
        });
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/one-shot")
                    .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}

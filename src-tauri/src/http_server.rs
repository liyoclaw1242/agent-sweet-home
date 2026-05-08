use crate::cache;
use crate::db::Db;
use crate::github::{Issue, PullRequest, Repo};
use crate::local_repo::{inspect_at, LocalRepoInspection};
use crate::settings::get_settings_inner;
use axum::{
    extract::{Path, State},
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    routing::get,
    Json, Router,
};
use serde::Serialize;
use std::net::SocketAddr;
use std::path::Path as StdPath;

#[derive(Clone)]
pub struct ServerCtx {
    pub db: Db,
    pub token: String,
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
            token: TOKEN.into(),
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
}

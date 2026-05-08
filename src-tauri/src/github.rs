use crate::cache;
use crate::db::Db;
use crate::settings::get_settings_inner;
use serde::{Deserialize, Serialize};
use tauri::State;

// ===== Repo =====

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Repo {
    pub id: u64,
    pub name: String,
    pub full_name: String,
    pub description: Option<String>,
    pub html_url: String,
    pub private: bool,
    pub default_branch: String,
    pub stargazers_count: u64,
    pub language: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
struct RawRepo {
    id: u64,
    name: String,
    full_name: String,
    description: Option<String>,
    html_url: String,
    private: bool,
    default_branch: String,
    stargazers_count: u64,
    language: Option<String>,
    updated_at: String,
}

impl From<RawRepo> for Repo {
    fn from(r: RawRepo) -> Self {
        Repo {
            id: r.id,
            name: r.name,
            full_name: r.full_name,
            description: r.description,
            html_url: r.html_url,
            private: r.private,
            default_branch: r.default_branch,
            stargazers_count: r.stargazers_count,
            language: r.language,
            updated_at: r.updated_at,
        }
    }
}

// ===== Issue =====

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub id: u64,
    pub number: u64,
    pub title: String,
    pub html_url: String,
    pub labels: Vec<Label>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Label {
    pub name: String,
    pub color: String,
}

#[derive(Debug, Deserialize)]
struct RawIssue {
    id: u64,
    number: u64,
    title: String,
    html_url: String,
    pull_request: Option<serde_json::Value>,
    labels: Vec<RawLabel>,
}

#[derive(Debug, Deserialize)]
struct RawLabel {
    name: String,
    color: String,
}

// ===== PullRequest =====

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub id: u64,
    pub number: u64,
    pub title: String,
    pub html_url: String,
    pub draft: bool,
}

#[derive(Debug, Deserialize)]
struct RawPr {
    id: u64,
    number: u64,
    title: String,
    html_url: String,
    draft: bool,
}

// ===== Helpers =====

async fn github_get_json<T: serde::de::DeserializeOwned>(
    url: &str,
    token: &str,
) -> Result<T, String> {
    let resp = reqwest::Client::new()
        .get(url)
        .header("Authorization", format!("token {}", token))
        .header("User-Agent", "agent-sweet-home")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("GitHub API {}: {}", status, body));
    }
    resp.json::<T>().await.map_err(|e| e.to_string())
}

fn read_token(db: &State<'_, Db>) -> Result<String, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let s = get_settings_inner(&conn).map_err(|e| e.to_string())?;
    if s.github_token.trim().is_empty() {
        return Err("GitHub credentials not configured".into());
    }
    Ok(s.github_token)
}

// ===== Commands =====

#[tauri::command]
pub async fn fetch_repos(db: State<'_, Db>) -> Result<Vec<Repo>, String> {
    let (username, token) = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        let s = get_settings_inner(&conn).map_err(|e| e.to_string())?;
        (s.github_username, s.github_token)
    };

    if username.trim().is_empty() || token.trim().is_empty() {
        return Err("GitHub credentials not configured".into());
    }

    let url = "https://api.github.com/user/repos?per_page=100&sort=updated";
    let raw: Vec<RawRepo> = github_get_json(url, &token).await?;
    let repos: Vec<Repo> = raw.into_iter().map(Repo::from).collect();

    {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        cache::replace_repos(&conn, &repos).map_err(|e| e.to_string())?;
    }

    Ok(repos)
}

#[tauri::command]
pub async fn fetch_issues(
    db: State<'_, Db>,
    repo_full_name: String,
) -> Result<Vec<Issue>, String> {
    let token = read_token(&db)?;
    let url = format!(
        "https://api.github.com/repos/{}/issues?state=open&per_page=50",
        repo_full_name
    );
    let raw: Vec<RawIssue> = github_get_json(&url, &token).await?;
    let issues: Vec<Issue> = raw
        .into_iter()
        .filter(|i| i.pull_request.is_none())
        .map(|i| Issue {
            id: i.id,
            number: i.number,
            title: i.title,
            html_url: i.html_url,
            labels: i
                .labels
                .into_iter()
                .map(|l| Label {
                    name: l.name,
                    color: l.color,
                })
                .collect(),
        })
        .collect();

    {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        cache::replace_issues(&conn, &repo_full_name, &issues).map_err(|e| e.to_string())?;
    }

    Ok(issues)
}

#[tauri::command]
pub async fn fetch_prs(
    db: State<'_, Db>,
    repo_full_name: String,
) -> Result<Vec<PullRequest>, String> {
    let token = read_token(&db)?;
    let url = format!(
        "https://api.github.com/repos/{}/pulls?state=open&per_page=50",
        repo_full_name
    );
    let raw: Vec<RawPr> = github_get_json(&url, &token).await?;
    let prs: Vec<PullRequest> = raw
        .into_iter()
        .map(|p| PullRequest {
            id: p.id,
            number: p.number,
            title: p.title,
            html_url: p.html_url,
            draft: p.draft,
        })
        .collect();

    {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        cache::replace_prs(&conn, &repo_full_name, &prs).map_err(|e| e.to_string())?;
    }

    Ok(prs)
}

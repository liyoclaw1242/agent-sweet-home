use crate::db::Db;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub github_username: String,
    pub github_token: String,
    pub local_base_path: String,
    pub workflow_path: String,
}

pub fn get_settings_inner(conn: &Connection) -> rusqlite::Result<Settings> {
    let mut stmt = conn.prepare("SELECT key, value FROM settings")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;

    let mut settings = Settings::default();
    for row in rows {
        let (k, v) = row?;
        match k.as_str() {
            "github_username" => settings.github_username = v,
            "github_token" => settings.github_token = v,
            "local_base_path" => settings.local_base_path = v,
            "workflow_path" => settings.workflow_path = v,
            _ => {}
        }
    }
    Ok(settings)
}

pub fn save_workflow_path_inner(conn: &Connection, path: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, datetime('now'))
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params!["workflow_path", path],
    )?;
    Ok(())
}

pub fn save_settings_inner(
    conn: &Connection,
    username: &str,
    token: &str,
    local_base_path: &str,
) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(
        "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, datetime('now'))
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )?;
    stmt.execute(params!["github_username", username])?;
    stmt.execute(params!["github_token", token])?;
    stmt.execute(params!["local_base_path", local_base_path])?;
    Ok(())
}

#[tauri::command]
pub fn get_settings(db: State<'_, Db>) -> Result<Settings, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    get_settings_inner(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_settings(
    db: State<'_, Db>,
    github_username: String,
    github_token: String,
    local_base_path: String,
) -> Result<(), String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    save_settings_inner(&conn, &github_username, &github_token, &local_base_path)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_workflow_path(db: State<'_, Db>, path: String) -> Result<(), String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    save_workflow_path_inner(&conn, &path).map_err(|e| e.to_string())
}

// ---- Per-repo workflow activation --------------------------------------

// ---- Per-repo workflow activation (opt-in, default off) ---------------

/// Returns the set of repo full names that have been explicitly opted in to
/// workflow scanning. Repos NOT in this set are skipped by the poll loop.
/// Default is empty → nothing is scanned until the user enables a repo.
pub fn get_active_repos_inner(conn: &Connection) -> std::collections::HashSet<String> {
    use rusqlite::OptionalExtension;
    let json: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'workflow_active_repos'",
            [],
            |r| r.get(0),
        )
        .optional()
        .unwrap_or(None);
    json.and_then(|j| serde_json::from_str::<Vec<String>>(&j).ok())
        .unwrap_or_default()
        .into_iter()
        .collect()
}

fn set_active_repos_inner(
    conn: &Connection,
    active: &std::collections::HashSet<String>,
) -> rusqlite::Result<()> {
    let mut list: Vec<&String> = active.iter().collect();
    list.sort();
    let json = serde_json::to_string(&list).unwrap_or_else(|_| "[]".into());
    conn.execute(
        "INSERT INTO settings (key, value, updated_at)
         VALUES ('workflow_active_repos', ?1, datetime('now'))
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![json],
    )?;
    Ok(())
}

/// Returns `true` when the repo has been explicitly opted in to workflow scanning.
#[tauri::command]
pub fn workflow_get_repo_active(db: State<'_, Db>, repo_full_name: String) -> bool {
    let conn = db.0.lock().unwrap_or_else(|e| e.into_inner());
    get_active_repos_inner(&conn).contains(&repo_full_name)
}

/// Add or remove `repo_full_name` from the opt-in list.
#[tauri::command]
pub fn workflow_set_repo_active(
    db: State<'_, Db>,
    repo_full_name: String,
    active: bool,
) -> Result<(), String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let mut set = get_active_repos_inner(&conn);
    if active {
        set.insert(repo_full_name);
    } else {
        set.remove(&repo_full_name);
    }
    set_active_repos_inner(&conn, &set).map_err(|e| e.to_string())
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
    fn workflow_repo_active_defaults_to_false() {
        let conn = fresh_conn();
        // No entry in DB → repo is NOT active (opt-in default)
        assert!(!get_active_repos_inner(&conn).contains("org/repo"));
    }

    #[test]
    fn workflow_repo_opt_in_round_trips() {
        let conn = fresh_conn();
        let mut set = get_active_repos_inner(&conn);
        set.insert("org/repo-a".into());
        set_active_repos_inner(&conn, &set).unwrap();

        let loaded = get_active_repos_inner(&conn);
        assert!(loaded.contains("org/repo-a"));
        assert!(!loaded.contains("org/repo-b"));

        // Opt out
        let mut set = get_active_repos_inner(&conn);
        set.remove("org/repo-a");
        set_active_repos_inner(&conn, &set).unwrap();
        assert!(!get_active_repos_inner(&conn).contains("org/repo-a"));
    }

    #[test]
    fn empty_db_returns_default_settings() {
        let conn = fresh_conn();
        let loaded = get_settings_inner(&conn).unwrap();
        assert_eq!(loaded, Settings::default());
    }

    #[test]
    fn save_then_load_round_trips_all_fields() {
        let conn = fresh_conn();
        save_settings_inner(&conn, "octocat", "ghp_secret", "/tmp/projects").unwrap();
        let loaded = get_settings_inner(&conn).unwrap();
        assert_eq!(loaded.github_username, "octocat");
        assert_eq!(loaded.github_token, "ghp_secret");
        assert_eq!(loaded.local_base_path, "/tmp/projects");
    }

    #[test]
    fn save_overrides_previous_values() {
        let conn = fresh_conn();
        save_settings_inner(&conn, "octocat", "old_token", "/old").unwrap();
        save_settings_inner(&conn, "octocat", "new_token", "/new").unwrap();
        let loaded = get_settings_inner(&conn).unwrap();
        assert_eq!(loaded.github_token, "new_token");
        assert_eq!(loaded.local_base_path, "/new");
    }

    #[test]
    fn workflow_path_round_trips_independently_of_other_settings() {
        let conn = fresh_conn();
        save_settings_inner(&conn, "octocat", "ghp", "/p").unwrap();
        save_workflow_path_inner(&conn, "/abs/wf.yaml").unwrap();
        let loaded = get_settings_inner(&conn).unwrap();
        assert_eq!(loaded.github_username, "octocat");
        assert_eq!(loaded.workflow_path, "/abs/wf.yaml");

        save_workflow_path_inner(&conn, "/abs/other.yaml").unwrap();
        let loaded = get_settings_inner(&conn).unwrap();
        assert_eq!(loaded.workflow_path, "/abs/other.yaml");
        assert_eq!(loaded.github_username, "octocat");
    }
}

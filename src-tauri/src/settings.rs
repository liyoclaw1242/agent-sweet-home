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
            _ => {}
        }
    }
    Ok(settings)
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
}

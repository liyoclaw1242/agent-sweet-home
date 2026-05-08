use rusqlite::{Connection, Result};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct Db(pub Arc<Mutex<Connection>>);

pub fn run_migrations(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS repos (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            full_name TEXT NOT NULL UNIQUE,
            description TEXT,
            html_url TEXT NOT NULL,
            private INTEGER NOT NULL,
            default_branch TEXT NOT NULL,
            language TEXT,
            stargazers_count INTEGER NOT NULL,
            updated_at TEXT NOT NULL,
            synced_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS issues (
            id INTEGER PRIMARY KEY,
            repo_full_name TEXT NOT NULL,
            number INTEGER NOT NULL,
            title TEXT NOT NULL,
            html_url TEXT NOT NULL,
            labels_json TEXT NOT NULL,
            synced_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_issues_repo ON issues(repo_full_name);

        CREATE TABLE IF NOT EXISTS prs (
            id INTEGER PRIMARY KEY,
            repo_full_name TEXT NOT NULL,
            number INTEGER NOT NULL,
            title TEXT NOT NULL,
            html_url TEXT NOT NULL,
            draft INTEGER NOT NULL,
            synced_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_prs_repo ON prs(repo_full_name);",
    )
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        run_migrations(&conn)?;
        Ok(Db(Arc::new(Mutex::new(conn))))
    }

    #[cfg(test)]
    pub fn from_connection(conn: Connection) -> Self {
        Db(Arc::new(Mutex::new(conn)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_create_all_tables_and_are_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        run_migrations(&conn).unwrap();

        let names: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert!(names.contains(&"settings".to_string()));
        assert!(names.contains(&"repos".to_string()));
        assert!(names.contains(&"issues".to_string()));
        assert!(names.contains(&"prs".to_string()));
    }
}

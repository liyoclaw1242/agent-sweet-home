use crate::github::{Issue, Label, PullRequest, Repo};
use rusqlite::{params, Connection};

// ===== Repos =====

pub fn replace_repos(conn: &Connection, repos: &[Repo]) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM repos", [])?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO repos (id, name, full_name, description, html_url, private,
                                default_branch, language, stargazers_count, updated_at, synced_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, datetime('now'))",
        )?;
        for r in repos {
            stmt.execute(params![
                r.id as i64,
                r.name,
                r.full_name,
                r.description,
                r.html_url,
                r.private as i64,
                r.default_branch,
                r.language,
                r.stargazers_count as i64,
                r.updated_at,
            ])?;
        }
    }
    tx.commit()
}

pub fn list_repos(conn: &Connection) -> rusqlite::Result<Vec<Repo>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, full_name, description, html_url, private,
                default_branch, language, stargazers_count, updated_at
         FROM repos ORDER BY updated_at DESC",
    )?;
    let rows = stmt.query_map([], row_to_repo)?;
    rows.collect()
}

pub fn get_repo_by_name(conn: &Connection, name: &str) -> rusqlite::Result<Option<Repo>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, full_name, description, html_url, private,
                default_branch, language, stargazers_count, updated_at
         FROM repos WHERE name = ?1 LIMIT 1",
    )?;
    match stmt.query_row(params![name], row_to_repo) {
        Ok(r) => Ok(Some(r)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
}

fn row_to_repo(row: &rusqlite::Row<'_>) -> rusqlite::Result<Repo> {
    Ok(Repo {
        id: row.get::<_, i64>(0)? as u64,
        name: row.get(1)?,
        full_name: row.get(2)?,
        description: row.get(3)?,
        html_url: row.get(4)?,
        private: row.get::<_, i64>(5)? != 0,
        default_branch: row.get(6)?,
        language: row.get(7)?,
        stargazers_count: row.get::<_, i64>(8)? as u64,
        updated_at: row.get(9)?,
    })
}

// ===== Issues =====

pub fn replace_issues(
    conn: &Connection,
    repo_full_name: &str,
    issues: &[Issue],
) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM issues WHERE repo_full_name = ?1",
        params![repo_full_name],
    )?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO issues (id, repo_full_name, number, title, html_url, labels_json, synced_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))",
        )?;
        for i in issues {
            let labels_json = serde_json::to_string(&i.labels).map_err(|e| {
                rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    e.to_string(),
                )))
            })?;
            stmt.execute(params![
                i.id as i64,
                repo_full_name,
                i.number as i64,
                i.title,
                i.html_url,
                labels_json,
            ])?;
        }
    }
    tx.commit()
}

pub fn list_issues(conn: &Connection, repo_full_name: &str) -> rusqlite::Result<Vec<Issue>> {
    let mut stmt = conn.prepare(
        "SELECT id, number, title, html_url, labels_json
         FROM issues WHERE repo_full_name = ?1 ORDER BY number DESC",
    )?;
    let rows = stmt.query_map(params![repo_full_name], |row| {
        let labels_json: String = row.get(4)?;
        let labels: Vec<Label> = serde_json::from_str(&labels_json).unwrap_or_default();
        Ok(Issue {
            id: row.get::<_, i64>(0)? as u64,
            number: row.get::<_, i64>(1)? as u64,
            title: row.get(2)?,
            html_url: row.get(3)?,
            labels,
        })
    })?;
    rows.collect()
}

// ===== PRs =====

pub fn replace_prs(
    conn: &Connection,
    repo_full_name: &str,
    prs: &[PullRequest],
) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM prs WHERE repo_full_name = ?1",
        params![repo_full_name],
    )?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO prs (id, repo_full_name, number, title, html_url, draft, synced_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))",
        )?;
        for p in prs {
            stmt.execute(params![
                p.id as i64,
                repo_full_name,
                p.number as i64,
                p.title,
                p.html_url,
                p.draft as i64,
            ])?;
        }
    }
    tx.commit()
}

pub fn list_prs(conn: &Connection, repo_full_name: &str) -> rusqlite::Result<Vec<PullRequest>> {
    let mut stmt = conn.prepare(
        "SELECT id, number, title, html_url, draft
         FROM prs WHERE repo_full_name = ?1 ORDER BY number DESC",
    )?;
    let rows = stmt.query_map(params![repo_full_name], |row| {
        Ok(PullRequest {
            id: row.get::<_, i64>(0)? as u64,
            number: row.get::<_, i64>(1)? as u64,
            title: row.get(2)?,
            html_url: row.get(3)?,
            draft: row.get::<_, i64>(4)? != 0,
        })
    })?;
    rows.collect()
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

    fn make_repo(id: u64, full: &str) -> Repo {
        let name = full.split('/').nth(1).unwrap().to_string();
        Repo {
            id,
            name,
            full_name: full.into(),
            description: None,
            html_url: format!("https://github.com/{full}"),
            private: false,
            default_branch: "main".into(),
            stargazers_count: 0,
            language: None,
            updated_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn replace_and_list_repos() {
        let conn = fresh_conn();
        replace_repos(&conn, &[make_repo(1, "u/a"), make_repo(2, "u/b")]).unwrap();
        let loaded = list_repos(&conn).unwrap();
        assert_eq!(loaded.len(), 2);
    }

    #[test]
    fn replace_clears_old_repos() {
        let conn = fresh_conn();
        replace_repos(&conn, &[make_repo(1, "u/a")]).unwrap();
        replace_repos(&conn, &[make_repo(2, "u/b")]).unwrap();
        let loaded = list_repos(&conn).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, 2);
    }

    #[test]
    fn get_repo_by_name_returns_match_or_none() {
        let conn = fresh_conn();
        replace_repos(&conn, &[make_repo(1, "u/alpha")]).unwrap();
        assert_eq!(
            get_repo_by_name(&conn, "alpha").unwrap().unwrap().id,
            1
        );
        assert!(get_repo_by_name(&conn, "missing").unwrap().is_none());
    }

    #[test]
    fn replace_and_list_issues_round_trips_labels() {
        let conn = fresh_conn();
        let issues = vec![Issue {
            id: 100,
            number: 42,
            title: "fix the thing".into(),
            html_url: "https://example.com".into(),
            labels: vec![Label {
                name: "p0".into(),
                color: "ff0000".into(),
            }],
        }];
        replace_issues(&conn, "u/a", &issues).unwrap();
        let loaded = list_issues(&conn, "u/a").unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].labels.len(), 1);
        assert_eq!(loaded[0].labels[0].name, "p0");
        assert_eq!(loaded[0].labels[0].color, "ff0000");
    }

    #[test]
    fn replace_issues_is_scoped_to_one_repo() {
        let conn = fresh_conn();
        let i1 = vec![Issue {
            id: 1,
            number: 1,
            title: "a".into(),
            html_url: "".into(),
            labels: vec![],
        }];
        let i2 = vec![Issue {
            id: 2,
            number: 2,
            title: "b".into(),
            html_url: "".into(),
            labels: vec![],
        }];
        replace_issues(&conn, "u/a", &i1).unwrap();
        replace_issues(&conn, "u/b", &i2).unwrap();
        // Wiping u/b does not touch u/a.
        replace_issues(&conn, "u/b", &[]).unwrap();
        assert_eq!(list_issues(&conn, "u/a").unwrap().len(), 1);
        assert_eq!(list_issues(&conn, "u/b").unwrap().len(), 0);
    }

    #[test]
    fn replace_and_list_prs() {
        let conn = fresh_conn();
        let prs = vec![PullRequest {
            id: 7,
            number: 7,
            title: "refactor".into(),
            html_url: "https://example.com".into(),
            draft: true,
        }];
        replace_prs(&conn, "u/a", &prs).unwrap();
        let loaded = list_prs(&conn, "u/a").unwrap();
        assert_eq!(loaded.len(), 1);
        assert!(loaded[0].draft);
    }
}

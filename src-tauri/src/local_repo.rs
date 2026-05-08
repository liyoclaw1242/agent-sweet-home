use crate::db::Db;
use crate::settings::get_settings_inner;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::State;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalRepoInspection {
    pub configured_base_path: String,
    pub repo_path: String,
    pub exists: bool,
    pub is_git_repo: bool,
    pub current_branch: Option<String>,
    pub is_clean: Option<bool>,
    pub dirty_files: Option<u32>,
    pub error: Option<String>,
}

pub fn inspect_at(base_path: &str, repo_name: &str) -> LocalRepoInspection {
    let mut result = LocalRepoInspection {
        configured_base_path: base_path.to_string(),
        repo_path: String::new(),
        exists: false,
        is_git_repo: false,
        current_branch: None,
        is_clean: None,
        dirty_files: None,
        error: None,
    };

    if base_path.trim().is_empty() {
        result.error = Some("Default local path not configured".into());
        return result;
    }

    let expanded = expand_tilde(base_path);
    let repo_path: PathBuf = expanded.join(repo_name);
    result.repo_path = repo_path.to_string_lossy().to_string();
    result.exists = repo_path.exists();
    if !result.exists {
        return result;
    }

    let git_dir = repo_path.join(".git");
    result.is_git_repo = git_dir.exists();
    if !result.is_git_repo {
        return result;
    }

    if let Some(branch) = read_current_branch(&repo_path) {
        result.current_branch = Some(branch);
    }

    match Command::new("git")
        .arg("-C")
        .arg(&repo_path)
        .arg("status")
        .arg("--porcelain")
        .output()
    {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let lines = stdout.lines().filter(|l| !l.is_empty()).count() as u32;
            result.dirty_files = Some(lines);
            result.is_clean = Some(lines == 0);
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            result.error = Some(format!("git status failed: {stderr}"));
        }
        Err(e) => {
            result.error = Some(format!("could not run git: {e}"));
        }
    }

    result
}

#[tauri::command]
pub fn inspect_local_repo(
    db: State<'_, Db>,
    repo_name: String,
) -> Result<LocalRepoInspection, String> {
    let base_path = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        let s = get_settings_inner(&conn).map_err(|e| e.to_string())?;
        s.local_base_path
    };
    Ok(inspect_at(&base_path, &repo_name))
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

fn read_current_branch(repo_path: &Path) -> Option<String> {
    let head = std::fs::read_to_string(repo_path.join(".git").join("HEAD")).ok()?;
    let head = head.trim();
    if let Some(rest) = head.strip_prefix("ref: refs/heads/") {
        Some(rest.to_string())
    } else {
        Some(head.chars().take(7).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_tilde_passes_absolute_path_through() {
        assert_eq!(expand_tilde("/tmp/foo"), PathBuf::from("/tmp/foo"));
    }

    #[test]
    fn expand_tilde_with_no_home_set_passes_through() {
        assert_eq!(expand_tilde("relative/path"), PathBuf::from("relative/path"));
    }

    #[test]
    fn inspect_at_with_empty_base_returns_configured_error() {
        let result = inspect_at("", "alpha");
        assert!(!result.exists);
        assert_eq!(
            result.error.as_deref(),
            Some("Default local path not configured")
        );
    }

    #[test]
    fn inspect_at_with_missing_repo_marks_not_existing() {
        let result = inspect_at("/tmp", "definitely-not-a-real-repo-9f8e7d");
        assert!(!result.exists);
        assert!(!result.is_git_repo);
        assert_eq!(
            result.repo_path,
            "/tmp/definitely-not-a-real-repo-9f8e7d"
        );
    }
}

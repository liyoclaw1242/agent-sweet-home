//! Git worktree allocation for roles that need write isolation.
//!
//! When a role declares `needs_worktree: true` (implementer, rlm-modeler),
//! the runtime carves a fresh sibling directory and a fresh branch off
//! `main` so the agent's commits stay isolated until `push_branch_and_pr`
//! delivers them. Cleanup happens after the on_result phase finishes,
//! whether or not the spawn succeeded.
//!
//! Convention (matches the v1 supervisor's wire format that the agent
//! prompts already document):
//!   - worktree path:  `<repo_root>-worktrees/spawn-<issue>-<unix_ts>`
//!   - branch name:    `spawn-<issue>-<unix_ts>`
//!
//! The agent receives the worktree as its `cwd` and is expected to commit
//! on the pre-created branch. It returns `branch` + `commit_sha` in its
//! structured output; `push_branch_and_pr` reads those to push + open a PR
//! against `<repo_root>` (the canonical clone owns the remote).

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::workflow::command::{run_capture, run_capture_full, CommandError};

#[derive(thiserror::Error, Debug)]
pub enum WorktreeError {
    #[error("repo path is not a directory: {0}")]
    BadRepoPath(PathBuf),
    #[error("git command failed: {0}")]
    Git(#[from] CommandError),
    #[error("failed to derive worktree parent dir from {0}")]
    NoParent(PathBuf),
}

/// One allocated worktree. Holds the canonical repo root + the carved
/// worktree path + the branch name we created. `cleanup()` is explicit so
/// callers control timing (it has to run *after* the on_result handlers
/// that read the worktree).
#[derive(Debug, Clone)]
pub struct WorktreeAllocation {
    /// Canonical clone (the one that owns `origin`).
    pub repo_root: PathBuf,
    /// The worktree directory the agent runs inside.
    pub worktree_path: PathBuf,
    /// Branch created by `git worktree add -b`.
    pub branch: String,
}

/// Carve a worktree and a fresh branch off the repo's current HEAD. Names
/// the branch `spawn-<issue>-<unix_ts>` so a single repo can host many
/// concurrent spawns without collision.
pub fn allocate(
    repo_root: &Path,
    issue_number: u64,
    base_ref: Option<&str>,
) -> Result<WorktreeAllocation, WorktreeError> {
    if !repo_root.is_dir() {
        return Err(WorktreeError::BadRepoPath(repo_root.to_path_buf()));
    }
    let parent = repo_root.parent().ok_or_else(|| WorktreeError::NoParent(repo_root.to_path_buf()))?;
    let repo_name = repo_root
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".to_string());

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let branch = format!("spawn-{issue_number}-{ts}");

    // Sibling dir: <parent>/<repo_name>-worktrees/<branch>
    let worktree_root = parent.join(format!("{repo_name}-worktrees"));
    std::fs::create_dir_all(&worktree_root).map_err(|e| {
        WorktreeError::Git(CommandError::Spawn(e))
    })?;
    let worktree_path = worktree_root.join(&branch);

    let base_clause = base_ref
        .map(|r| format!(" {}", shell_quote(r)))
        .unwrap_or_default();
    let cmd = format!(
        "cd {repo} && git worktree add -b {branch} {path}{base}",
        repo = shell_quote(&repo_root.to_string_lossy()),
        branch = shell_quote(&branch),
        path = shell_quote(&worktree_path.to_string_lossy()),
        base = base_clause,
    );
    run_capture(&cmd)?;

    Ok(WorktreeAllocation {
        repo_root: repo_root.to_path_buf(),
        worktree_path,
        branch,
    })
}

/// Best-effort tear-down: remove the worktree dir and prune the registry.
/// Always tries `--force` since the agent may have left an unmerged
/// branch or stale lock; we don't want a half-cleaned worktree to block
/// future spawns. Errors are logged but never propagated — the work is
/// already delivered (or already failed).
pub fn cleanup(alloc: &WorktreeAllocation) {
    let cmd = format!(
        "cd {repo} && git worktree remove --force {path} 2>&1 || true; git worktree prune 2>&1 || true",
        repo = shell_quote(&alloc.repo_root.to_string_lossy()),
        path = shell_quote(&alloc.worktree_path.to_string_lossy()),
    );
    if let Err(e) = run_capture_full(&cmd) {
        eprintln!(
            "worktree cleanup warning ({}): {}",
            alloc.worktree_path.display(),
            e
        );
    }
}

fn shell_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn make_temp_git_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path();
        let run = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(path)
                .output()
                .expect("git")
        };
        let _ = run(&["init", "-b", "main"]);
        let _ = run(&["config", "user.email", "test@example.com"]);
        let _ = run(&["config", "user.name", "tester"]);
        std::fs::write(path.join("README.md"), "hello").unwrap();
        let _ = run(&["add", "."]);
        let _ = run(&["commit", "-m", "init"]);
        dir
    }

    #[test]
    fn allocate_then_cleanup_round_trip() {
        if std::process::Command::new("git").arg("--version").output().is_err() {
            eprintln!("skip: git not available");
            return;
        }
        let dir = make_temp_git_repo();
        let alloc = allocate(dir.path(), 42, None).expect("allocate");
        assert!(alloc.worktree_path.is_dir());
        assert!(alloc.branch.starts_with("spawn-42-"));
        // README from the seed commit must be visible inside the worktree
        assert!(alloc.worktree_path.join("README.md").exists());
        cleanup(&alloc);
        assert!(!alloc.worktree_path.exists(), "worktree dir should be gone after cleanup");
    }

    #[test]
    fn allocate_errors_when_repo_path_missing() {
        let err = allocate(Path::new("/no/such/path"), 1, None).unwrap_err();
        assert!(matches!(err, WorktreeError::BadRepoPath(_)));
    }
}

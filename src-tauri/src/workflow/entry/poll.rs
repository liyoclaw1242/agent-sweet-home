//! Poll mode — every `interval_sec`, run a sweep:
//!   1. `repo_source.command`  → [{repo, path}]
//!   2. for each repo: `issue_source.command{repo=...}` → [issue, ...]
//!   3. for each issue (capped by `max_in_flight`): dispatch + execute
//!
//! Stateless across ticks; the runtime provides a shutdown hook so Tauri
//! can stop the loop on app exit.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::sync::Semaphore;

use super::{EntryError, IssueSource, RepoRef};
use crate::workflow::command::{
    render_template, run_capture, run_capture_json, shell_quote, CommandError,
};
use crate::workflow::expr::IssueSnapshot;
use crate::workflow::runtime::{parse_body_markers, WorkflowRuntime};
use crate::workflow::spec::PollConfig;

/// Raw issue shape returned by `gh issue list --json …`. The `gh` CLI
/// flattens labels into objects, so we deserialize into the same shape and
/// flatten in-Rust to `IssueSnapshot`.
#[derive(Debug, Deserialize)]
struct RawIssue {
    number: u64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    labels: Vec<RawLabel>,
}

#[derive(Debug, Deserialize)]
struct RawLabel {
    name: String,
}

impl RawIssue {
    fn into_snapshot(self) -> IssueSnapshot {
        IssueSnapshot {
            number: self.number,
            title: self.title,
            // gh sometimes returns labels but ignores body in some queries;
            // body markers parsed below tolerate empty input.
            markers: parse_body_markers(&self.body),
            body: self.body,
            state: self.state.to_lowercase(),
            labels: self.labels.into_iter().map(|l| l.name).collect(),
        }
    }
}

/// `IssueSource` impl that resolves repos + issues by shelling out to the
/// commands declared in `PollConfig`.
pub struct PollSource<'a> {
    pub cfg: &'a PollConfig,
}

#[async_trait]
impl<'a> IssueSource for PollSource<'a> {
    async fn fetch_repos(&self) -> Result<Vec<RepoRef>, EntryError> {
        let src = self.cfg.repo_source.as_ref()
            .expect("fetch_repos called without repo_source configured in YAML");
        let cmd = render_template(&src.command, &HashMap::new())?;
        let repos: Vec<RepoRef> = run_capture_json(&cmd)?;
        Ok(repos)
    }

    async fn fetch_issues(&self, repo: &str) -> Result<Vec<IssueSnapshot>, EntryError> {
        let mut vars = HashMap::new();
        vars.insert("repo", repo);
        let cmd = render_template(&self.cfg.issue_source.command, &vars)?;
        let raw: Vec<RawIssue> = run_capture_json(&cmd)?;
        Ok(raw.into_iter().map(RawIssue::into_snapshot).collect())
    }

    async fn fetch_one(&self, repo: &str, issue_number: u64) -> Result<IssueSnapshot, EntryError> {
        let mut vars = HashMap::new();
        let num_string = issue_number.to_string();
        vars.insert("repo", repo);
        vars.insert("issue_number", num_string.as_str());
        let cmd = render_template(&self.cfg.issue_source.command, &vars)?;
        let raw: RawIssue = run_capture_json(&cmd)?;
        Ok(raw.into_snapshot())
    }
}

/// Run the poll loop until `shutdown` resolves. Each tick:
/// - calls `source.fetch_repos()`, then `source.fetch_issues(repo)` per
///   repo, then dispatches each issue concurrently (bounded by
///   `cfg.max_in_flight` via a `Semaphore`),
/// - waits for all in-flight dispatches before sleeping to the next tick.
pub async fn run_poll_loop(
    cfg: &PollConfig,
    runtime: Arc<WorkflowRuntime>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
    pause: Arc<std::sync::atomic::AtomicBool>,
) -> Result<(), EntryError> {
    let semaphore = Arc::new(Semaphore::new(cfg.max_in_flight.max(1)));
    let mut interval = tokio::time::interval(Duration::from_secs(cfg.interval_sec.max(1)));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    return Ok(());
                }
            }
            _ = interval.tick() => {
                if pause.load(std::sync::atomic::Ordering::Relaxed) {
                    continue;
                }
                if let Err(e) = run_one_tick(cfg, runtime.clone(), semaphore.clone()).await {
                    eprintln!("workflow poll tick failed: {e}");
                }
            }
        }
    }
}

async fn run_one_tick(
    cfg: &PollConfig,
    runtime: Arc<WorkflowRuntime>,
    semaphore: Arc<Semaphore>,
) -> Result<(), EntryError> {
    // Discover repos — either via the YAML repo_source command (backward
    // compat) or from the app's sidebar cache filtered to repos with a valid
    // local git clone.
    let source = PollSource { cfg };
    let all_repos: Vec<RepoRef> = if cfg.repo_source.is_some() {
        source.fetch_repos().await?
    } else {
        system_repos(&runtime.db)
    };

    // Keep only repos the user has explicitly opted in to workflow scanning.
    let active_set = {
        let conn = runtime.db.0.lock().unwrap_or_else(|e| e.into_inner());
        crate::settings::get_active_repos_inner(&conn)
    };
    let repos: Vec<_> = all_repos
        .into_iter()
        .filter(|r| active_set.contains(&r.repo))
        .collect();
    let mut tasks = Vec::new();
    for repo in repos {
        let issues = match source.fetch_issues(&repo.repo).await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("fetch issues for {} failed: {}", repo.repo, e);
                continue;
            }
        };
        for issue in issues {
            let permit = match semaphore.clone().acquire_owned().await {
                Ok(p) => p,
                Err(_) => return Ok(()), // semaphore closed → caller shutting down
            };
            let runtime_for_task = runtime.clone();
            let repo_for_task = repo.clone();
            let issue_for_task = issue;
            let task = tokio::spawn(async move {
                let _permit = permit; // released on task drop
                let outcome = runtime_for_task
                    .dispatch_one(&repo_for_task, issue_for_task.clone())
                    .await;
                match outcome {
                    Ok(o) => {
                        eprintln!(
                            "workflow: {}#{} → {:?}",
                            repo_for_task.repo, issue_for_task.number, o
                        );
                    }
                    Err(e) => {
                        eprintln!(
                            "workflow: {}#{} dispatch error: {}",
                            repo_for_task.repo, issue_for_task.number, e
                        );
                        // Quarantine the issue so the next poll tick doesn't
                        // re-dispatch the same failing case (each retry costs
                        // a real $$ spawn). Add `human-review`, drop
                        // `status:ready`. Best-effort — failures here are
                        // logged but don't escalate.
                        if let Err(qe) = quarantine_issue(
                            &repo_for_task.repo,
                            issue_for_task.number,
                        ) {
                            eprintln!(
                                "workflow: {}#{} quarantine failed: {}",
                                repo_for_task.repo, issue_for_task.number, qe
                            );
                        }
                    }
                }
            });
            tasks.push(task);
        }
    }
    for task in tasks {
        let _ = task.await;
    }
    Ok(())
}

/// Mark an issue as failed-needs-human after a dispatch error so the next
/// poll tick doesn't re-fire the same failing case (each retry costs a
/// real spawn). Adds `human-review` only; we do NOT remove a "ready" status
/// label because workflows in the wild may use different ready-state
/// conventions (`status:approved`, `status:ready`, etc.) and a hardcoded
/// `--remove-label` causes the whole `gh issue edit` call to fail
/// atomically when the label isn't present. The downstream dispatch rule
/// that filters `has_label: "human-review"` is what actually halts the
/// retry loop. Best-effort — the caller logs failures.
fn quarantine_issue(repo: &str, issue_number: u64) -> Result<(), CommandError> {
    let cmd = format!(
        "gh issue edit {num} --repo {repo} --add-label {hr}",
        num = issue_number,
        repo = shell_quote(repo),
        hr = shell_quote("human-review"),
    );
    run_capture(&cmd)?;
    Ok(())
}

/// Discover repos from the app's sidebar cache. Each repo is included only
/// when its local path exists and contains a `.git` directory.
fn system_repos(db: &crate::db::Db) -> Vec<RepoRef> {
    let (repos, base_path) = {
        let conn = db.0.lock().unwrap_or_else(|e| e.into_inner());
        let repos = crate::cache::list_repos(&conn).unwrap_or_default();
        let settings = crate::settings::get_settings_inner(&conn).unwrap_or_default();
        (repos, settings.local_base_path)
    };

    let effective_base = if base_path.trim().is_empty() {
        crate::local_repo::DEFAULT_BASE_PATH.to_string()
    } else {
        base_path
    };

    repos
        .into_iter()
        .filter_map(|r| {
            let path = resolve_path(&effective_base, &r.name);
            if path.exists() && path.join(".git").exists() {
                Some(RepoRef {
                    repo: r.full_name,
                    path: path.to_string_lossy().to_string(),
                })
            } else {
                eprintln!(
                    "workflow: skipping {} — no local git clone at {}",
                    r.full_name,
                    path.display()
                );
                None
            }
        })
        .collect()
}

fn resolve_path(base: &str, repo_name: &str) -> PathBuf {
    let expanded = if let Some(rest) = base.strip_prefix("~/") {
        std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join(rest))
            .unwrap_or_else(|| PathBuf::from(base))
    } else {
        PathBuf::from(base)
    };
    expanded.join(repo_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_issue_into_snapshot_lowercases_state() {
        let raw = RawIssue {
            number: 1,
            title: "t".into(),
            body: "<!-- subdomain: x -->".into(),
            state: "OPEN".into(),
            labels: vec![RawLabel {
                name: "agent:foo".into(),
            }],
        };
        let snap = raw.into_snapshot();
        assert_eq!(snap.state, "open");
        assert_eq!(snap.labels, vec!["agent:foo".to_string()]);
        assert_eq!(
            snap.markers.get("subdomain").map(String::as_str),
            Some("x")
        );
    }
}

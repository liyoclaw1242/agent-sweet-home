//! Poll mode — every `interval_sec`, run a sweep:
//!   1. `repo_source.command`  → [{repo, path}]
//!   2. for each repo: `issue_source.command{repo=...}` → [issue, ...]
//!   3. for each issue (capped by `max_in_flight`): dispatch + execute
//!
//! Stateless across ticks; the runtime provides a shutdown hook so Tauri
//! can stop the loop on app exit.

use super::{EntryError, IssueSource, RepoRef};
use crate::workflow::expr::IssueSnapshot;
use crate::workflow::spec::PollConfig;
use async_trait::async_trait;

/// `IssueSource` impl that resolves repos + issues by shelling out to the
/// commands declared in `PollConfig`.
pub struct PollSource<'a> {
    pub cfg: &'a PollConfig,
}

#[async_trait]
impl<'a> IssueSource for PollSource<'a> {
    async fn fetch_repos(&self) -> Result<Vec<RepoRef>, EntryError> {
        todo!("Phase 2: render_template(self.cfg.repo_source.command, &[]) → run_capture_json")
    }

    async fn fetch_issues(&self, _repo: &str) -> Result<Vec<IssueSnapshot>, EntryError> {
        todo!(
            "Phase 2: render_template(self.cfg.issue_source.command, &[(\"repo\", repo)]) \
             → run_capture_json::<Vec<RawIssue>> → map to IssueSnapshot (parse markers)"
        )
    }

    async fn fetch_one(&self, _repo: &str, _issue_number: u64) -> Result<IssueSnapshot, EntryError> {
        Err(EntryError::UnsupportedMode(crate::workflow::spec::EntryMode::Poll))
    }
}

/// Run the poll loop until `shutdown` resolves. Each tick:
/// - calls `source.fetch_repos()`, then `source.fetch_issues(repo)` per
///   repo, then dispatches each issue via the runtime (Phase 2),
/// - bounds concurrent dispatches to `cfg.max_in_flight` via a `JoinSet`
///   + semaphore.
pub async fn run_poll_loop(
    _cfg: &PollConfig,
    _shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<(), EntryError> {
    todo!(
        "Phase 2: tokio::time::interval(cfg.interval_sec) loop; tokio::select! on shutdown; \
         each tick build PollSource, fetch_repos, JoinSet-bounded fan-out per repo"
    )
}

//! Manual mode — direct entry points for CLI / Tauri commands / tests:
//!   - `run_one(repo, issue_number)`  — single issue dispatch (hot-fix flow)
//!   - `run_sweep(repo)`              — one-shot sweep over a repo, like a
//!                                       single poll tick but bounded
//!
//! Reuses the same `IssueSource` shape as poll/webhook so the executor side
//! doesn't care which mode triggered it.

use super::{EntryError, IssueSource, RepoRef};
use crate::workflow::expr::IssueSnapshot;
use crate::workflow::spec::ManualConfig;
use async_trait::async_trait;

pub struct ManualSource<'a> {
    pub cfg: &'a ManualConfig,
}

#[async_trait]
impl<'a> IssueSource for ManualSource<'a> {
    async fn fetch_repos(&self) -> Result<Vec<RepoRef>, EntryError> {
        Err(EntryError::UnsupportedMode(crate::workflow::spec::EntryMode::Manual))
    }

    async fn fetch_issues(&self, _repo: &str) -> Result<Vec<IssueSnapshot>, EntryError> {
        // Manual sweep can fall back to the poll issue_source if wired
        // explicitly in Phase 2; manual.issue_source itself is single-issue.
        Err(EntryError::UnsupportedMode(crate::workflow::spec::EntryMode::Manual))
    }

    async fn fetch_one(&self, _repo: &str, _issue_number: u64) -> Result<IssueSnapshot, EntryError> {
        todo!(
            "Phase 2: render_template(self.cfg.issue_source.command, \
             &[(\"repo\", repo), (\"issue_number\", &n.to_string())]) → run_capture_json"
        )
    }
}

/// Single-issue dispatch — used by Tauri command + CLI hot-fix flow + tests.
pub async fn run_one(
    _cfg: &ManualConfig,
    _repo: &str,
    _issue_number: u64,
) -> Result<(), EntryError> {
    todo!(
        "Phase 2: ManualSource::fetch_one → Workflow::dispatch → \
         spawn / apply_on_result / apply_degrade pipeline"
    )
}

/// Full-repo sweep — the poll tick logic, executed once. Reuses the
/// `PollConfig.issue_source` if available; otherwise errors. (Manual
/// itself is single-issue; a sweep needs a list source.)
pub async fn run_sweep(_repo: &str) -> Result<(), EntryError> {
    todo!(
        "Phase 2: borrow PollConfig.issue_source from Workflow.entry.poll, \
         iterate issues, dispatch each — but cap at max_in_flight=1 since \
         this is a manual debug op"
    )
}

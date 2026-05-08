//! Entry — the layer that decides *when* to dispatch. Three modes share the
//! same `dispatch:` + `on_result:` tables from `Workflow`:
//!
//!   - `poll`    — periodic sweep (production default)
//!   - `webhook` — push-driven, single-issue (off by default)
//!   - `manual`  — CLI / Tauri command (run_one / run_sweep)
//!
//! `IssueSource` is the abstraction each mode plugs into so the dispatcher
//! never grows direct knowledge of `gh` / GitHub API / mock fixtures. The
//! actual command lookup goes through `command::render_template` →
//! `command::run_capture_json`.

pub mod manual;
pub mod poll;
pub mod webhook;

use crate::workflow::expr::IssueSnapshot;
use crate::workflow::spec::EntryMode;
use async_trait::async_trait;

/// Resolved repo entry produced by `repo_source.command`.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct RepoRef {
    pub repo: String,
    pub path: String,
}

#[derive(thiserror::Error, Debug)]
pub enum EntryError {
    #[error("command error: {0}")]
    Command(#[from] crate::workflow::command::CommandError),
    #[error("mode {0:?} requested but config block is missing")]
    ModeNotConfigured(EntryMode),
    #[error("unsupported entry mode for this op: {0:?}")]
    UnsupportedMode(EntryMode),
}

/// What every entry-mode driver provides to the dispatcher. `fetch_one`
/// supports webhook / manual single-issue ops; `fetch_repos` + `fetch_issues`
/// drives the poll sweep. Implementations are stateless (each call shells
/// out via `command.rs`).
#[async_trait]
pub trait IssueSource: Send + Sync {
    async fn fetch_repos(&self) -> Result<Vec<RepoRef>, EntryError>;
    async fn fetch_issues(&self, repo: &str) -> Result<Vec<IssueSnapshot>, EntryError>;
    async fn fetch_one(&self, repo: &str, issue_number: u64) -> Result<IssueSnapshot, EntryError>;
}

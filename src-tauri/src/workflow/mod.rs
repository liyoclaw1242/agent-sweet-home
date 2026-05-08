//! Workflow engine — declarative YAML+Jinja runtime.
//!
//! Layout:
//! - `spec`      — serde structs (Workflow, EntryConfig, RoleConfig, …).
//! - `entry/*`   — three entry modes (poll / webhook / manual) sharing the
//!                 `IssueSource` trait. Decides *when* to dispatch.
//! - `command`   — shared shell-template renderer + spawner.
//! - `expr`      — minijinja wrapper for predicates / template rendering.
//! - `dispatch`  — pure fn `(IssueSnapshot, [DispatchRule]) → Directive`.
//! - `result`    — on_result handlers + degrade fallback + unblock pass.
//! - `spawn`     — bridge from RoleConfig → one_shot::start_run + log
//!                 scrape for structured output.
//! - `runtime`   — orchestrator that ties dispatch + spawn + on_result.

pub mod command;
pub mod dispatch;
pub mod entry;
pub mod expr;
pub mod result;
pub mod runtime;
pub mod spawn;
pub mod spec;

use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(thiserror::Error, Debug)]
pub enum LoadError {
    #[error("read {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("parse {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_yaml::Error,
    },
}

/// Load + parse a workflow YAML from disk. Pure deserialization — no
/// side effects, no entry threads spawned. Use `start` to actually drive
/// the dispatch loop.
pub fn load(path: &Path) -> Result<spec::Workflow, LoadError> {
    let text = std::fs::read_to_string(path).map_err(|source| LoadError::Read {
        path: path.display().to_string(),
        source,
    })?;
    spec::Workflow::from_yaml(&text).map_err(|source| LoadError::Parse {
        path: path.display().to_string(),
        source,
    })
}

/// Spawn one tokio task per active entry mode declared in the workflow's
/// `entry.modes` list and return immediately. Each task watches the same
/// `shutdown` channel; flipping the channel to `true` cleanly stops every
/// entry driver.
///
/// Caller owns the `Arc<WorkflowRuntime>`; this function clones it per
/// driver so each entry mode owns its handle.
pub fn start(
    runtime: Arc<runtime::WorkflowRuntime>,
    shutdown_rx: tokio::sync::watch::Receiver<bool>,
) -> Vec<tokio::task::JoinHandle<()>> {
    let mut handles = Vec::new();
    let modes = runtime.wf.entry.modes.clone();
    for mode in modes {
        match mode {
            spec::EntryMode::Poll => {
                let Some(_cfg) = runtime.wf.entry.poll.as_ref() else {
                    eprintln!("workflow: entry.modes lists `poll` but entry.poll is missing");
                    continue;
                };
                let runtime_clone = runtime.clone();
                let shutdown_clone = shutdown_rx.clone();
                let handle = tokio::spawn(async move {
                    let cfg = runtime_clone
                        .wf
                        .entry
                        .poll
                        .as_ref()
                        .expect("poll cfg present at start time");
                    if let Err(e) = entry::poll::run_poll_loop(
                        cfg,
                        runtime_clone.clone(),
                        shutdown_clone,
                    )
                    .await
                    {
                        eprintln!("workflow poll loop ended with error: {e}");
                    }
                });
                handles.push(handle);
            }
            spec::EntryMode::Manual => {
                // Manual = HTTP / Tauri command driven; no background loop.
            }
            spec::EntryMode::Webhook => {
                eprintln!("workflow: webhook entry mode is not yet implemented");
            }
        }
    }
    handles
}

/// Resolve a workflow YAML path to its containing directory; this is the
/// directory that `RoleConfig::system_prompt_file` paths are relative to.
pub fn workflow_dir_of(yaml_path: &Path) -> PathBuf {
    yaml_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

#[allow(unused_imports)]
pub use dispatch::{dispatch, eval_predicate, DispatchError};
#[allow(unused_imports)]
pub use expr::{ExprContext, ExprEngine, ExprError, IssueSnapshot};
#[allow(unused_imports)]
pub use result::{apply_degrade, apply_on_result, apply_unblock_pass, ResultError};
#[allow(unused_imports)]
pub use runtime::{DispatchOutcome, RuntimeError, WorkflowRuntime};
#[allow(unused_imports)]
pub use spec::{EntryMode, Workflow};

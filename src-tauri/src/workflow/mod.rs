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
//!
//! Phase 1 wires types only. Phase 2 lands runtime bodies.

pub mod command;
pub mod dispatch;
pub mod entry;
pub mod expr;
pub mod result;
pub mod spec;

use std::path::Path;

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
/// side effects, no entry threads spawned. Use `Workflow::run` (Phase 2)
/// to actually start poll/webhook loops.
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

// Phase 1 only: nothing inside the crate consumes these yet, but later
// phases (runtime, HTTP routes, UI bridge) all use them. Quiet the warning
// until Phase 4 wires the runtime in.
#[allow(unused_imports)]
pub use dispatch::{dispatch, eval_predicate, DispatchError};
#[allow(unused_imports)]
pub use expr::{ExprContext, ExprEngine, ExprError, IssueSnapshot};
#[allow(unused_imports)]
pub use result::{apply_degrade, apply_on_result, apply_unblock_pass, ResultError};
#[allow(unused_imports)]
pub use spec::{EntryMode, Workflow};

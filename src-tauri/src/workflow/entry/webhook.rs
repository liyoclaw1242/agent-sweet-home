//! Webhook mode — listen for GitHub webhooks (issues / issue_comment /
//! label) and dispatch the affected issue. Off by default
//! (`webhook.enabled: false` in YAML); the runtime should not bind to the
//! port unless enabled is true.
//!
//! Phase-1 skeleton only: signature verification, event filtering and
//! hand-off to the dispatcher all land in Phase 2 alongside the GitHub
//! writer.

use super::{EntryError, IssueSource, RepoRef};
use crate::workflow::expr::IssueSnapshot;
use crate::workflow::spec::WebhookConfig;
use async_trait::async_trait;

pub struct WebhookSource<'a> {
    pub cfg: &'a WebhookConfig,
}

#[async_trait]
impl<'a> IssueSource for WebhookSource<'a> {
    async fn fetch_repos(&self) -> Result<Vec<RepoRef>, EntryError> {
        Err(EntryError::UnsupportedMode(crate::workflow::spec::EntryMode::Webhook))
    }

    async fn fetch_issues(&self, _repo: &str) -> Result<Vec<IssueSnapshot>, EntryError> {
        Err(EntryError::UnsupportedMode(crate::workflow::spec::EntryMode::Webhook))
    }

    async fn fetch_one(&self, _repo: &str, _issue_number: u64) -> Result<IssueSnapshot, EntryError> {
        todo!(
            "Phase 2: render_template(self.cfg.issue_source.command, \
             &[(\"repo\", repo), (\"issue_number\", &n.to_string())]) → run_capture_json"
        )
    }
}

/// Bind axum to `cfg.listen` and serve the webhook router until `shutdown`.
/// **Early-returns Ok(()) when `cfg.enabled == false`** — the runtime calls
/// this unconditionally, the no-op gate lives here.
pub async fn run_webhook_listener(
    cfg: &WebhookConfig,
    _shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<(), EntryError> {
    if !cfg.enabled {
        return Ok(());
    }
    todo!(
        "Phase 2: axum router with POST / handler that verifies HMAC against \
         std::env::var(cfg.secret_env), filters by cfg.events, then dispatches \
         via WebhookSource::fetch_one + Workflow::dispatch + apply_on_result"
    )
}

mod cache;
mod db;
pub mod graph;
mod github;
mod http_server;
mod local_repo;
mod one_shot;
mod settings;
mod terminal;
pub mod workflow;

use std::sync::Arc;
use tauri::Manager;

/// Holds the watch sender that, when set to `true`, asks every entry-mode
/// driver to drain in-flight dispatches and exit cleanly.
struct WorkflowShutdown(tokio::sync::watch::Sender<bool>);

/// Pauses the poll loop without killing it. `true` = paused, `false` = running.
pub struct WorkflowPause(pub Arc<std::sync::atomic::AtomicBool>);

#[tauri::command]
fn workflow_set_running(
    pause: tauri::State<'_, WorkflowPause>,
    running: bool,
) -> bool {
    pause.0.store(!running, std::sync::atomic::Ordering::Relaxed);
    running
}

#[tauri::command]
fn workflow_is_running(pause: tauri::State<'_, WorkflowPause>) -> bool {
    !pause.0.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let app_dir = app
                .path()
                .app_data_dir()
                .expect("failed to resolve app data dir");
            std::fs::create_dir_all(&app_dir).ok();
            let db_path = app_dir.join("agent-sweet-home.db");
            let db = db::Db::open(&db_path).expect("failed to open database");
            app.manage(db.clone());

            let registry = terminal::Registry::new();
            app.manage(registry.clone());

            let one_shot_state = one_shot::OneShotState::new();
            app.manage(one_shot_state.clone());

            // Shared workflow status — starts empty, written after workflow load.
            // Shared between the HTTP server and the Tauri command.
            let wf_status: Arc<std::sync::RwLock<workflow::WorkflowStatus>> =
                Arc::new(std::sync::RwLock::new(workflow::WorkflowStatus {
                    path: String::new(),
                    exists: false,
                    loaded: false,
                    error: None,
                }));

            // Spawn a localhost-only HTTP API for external CLIs / agents to query
            // the cached repo list and details. The auth token + port are written
            // to <app_data_dir>/server.json (0600 on unix).
            let token = uuid::Uuid::new_v4().to_string();
            let server_ctx = http_server::ServerCtx {
                db: db.clone(),
                registry: registry.clone(),
                one_shot: one_shot_state.clone(),
                app_handle: Some(app.handle().clone()),
                token,
                workflow_status: Some(wf_status.clone()),
            };
            let server_app_dir = app_dir.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) =
                    http_server::bind_and_serve(server_ctx, &server_app_dir).await
                {
                    eprintln!("agent-sweet-home: HTTP API stopped with error: {e}");
                }
            });

            // ---- Workflow runtime --------------------------------------
            //
            // Resolve the workflow YAML path with a 3-tier priority:
            //   1. WORKFLOW_FILE env var (highest — explicit per-launch override)
            //   2. settings.workflow_path stored in the SQLite settings table
            //      (set via the WorkflowView "Save path" input)
            //   3. <app_data_dir>/workflow.yaml (lowest — convention fallback)
            // If the resolved path doesn't exist we log + skip; the rest of
            // the app still works without a declarative workflow loaded.
            let db_workflow_path = {
                let conn = db.0.lock().expect("db lock poisoned");
                settings::get_settings_inner(&conn)
                    .ok()
                    .map(|s| s.workflow_path)
                    .filter(|p| !p.is_empty())
            };
            let workflow_path = std::env::var("WORKFLOW_FILE")
                .ok()
                .filter(|s| !s.is_empty())
                .or(db_workflow_path)
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| app_dir.join("workflow.yaml"));

            // Pause flag shared between the poll loop and the toggle command.
            let pause_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
            app.manage(WorkflowPause(pause_flag.clone()));

            let mut wf_loaded = false;
            let mut wf_error: Option<String> = None;
            let wf_exists = workflow_path.exists();
            if wf_exists {
                match workflow::load(&workflow_path) {
                    Ok(wf) => {
                        let runtime = Arc::new(workflow::WorkflowRuntime::new(
                            wf,
                            workflow::workflow_dir_of(&workflow_path),
                            app.handle().clone(),
                            db.clone(),
                            one_shot_state.clone(),
                        ));
                        let (tx, rx) = tokio::sync::watch::channel(false);
                        app.manage(WorkflowShutdown(tx));
                        let _handles = workflow::start(runtime, rx, pause_flag.clone());
                        eprintln!("workflow: loaded {}", workflow_path.display());
                        wf_loaded = true;
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        eprintln!("workflow: failed to load {}: {msg}", workflow_path.display());
                        wf_error = Some(msg);
                    }
                }
            } else {
                eprintln!(
                    "workflow: no workflow.yaml at {} (set WORKFLOW_FILE to override)",
                    workflow_path.display()
                );
            }
            *wf_status.write().unwrap() = workflow::WorkflowStatus {
                path: workflow_path.display().to_string(),
                exists: wf_exists,
                loaded: wf_loaded,
                error: wf_error,
            };
            // Expose the Arc as Tauri-managed state (the workflow_status command
            // reads through it).
            app.manage(wf_status.clone());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            settings::get_settings,
            settings::save_settings,
            settings::save_workflow_path,
            github::fetch_repos,
            github::fetch_issues,
            github::fetch_prs,
            local_repo::inspect_local_repo,
            terminal::pty_create,
            terminal::pty_write,
            terminal::pty_resize,
            terminal::pty_kill,
            terminal::pty_list,
            terminal::pty_get,
            one_shot::one_shot_start,
            one_shot::one_shot_list,
            one_shot::one_shot_get,
            one_shot::one_shot_log,
            one_shot::one_shot_kill,
            workflow::workflow_status,
            graph::graph_state_cmd,
            graph::graph_blocking_cmd,
            graph::graph_run_events_cmd,
            workflow_set_running,
            workflow_is_running,
            settings::workflow_get_repo_active,
            settings::workflow_set_repo_active,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

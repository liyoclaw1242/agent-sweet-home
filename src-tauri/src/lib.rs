mod cache;
mod db;
mod github;
mod http_server;
mod local_repo;
mod one_shot;
mod settings;
mod terminal;
pub mod workflow;

use tauri::Manager;

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

            // Spawn a localhost-only HTTP API for external CLIs / agents to query
            // the cached repo list and details. The auth token + port are written
            // to <app_data_dir>/server.json (0600 on unix).
            let token = uuid::Uuid::new_v4().to_string();
            let server_ctx = http_server::ServerCtx {
                db: db.clone(),
                registry: registry.clone(),
                one_shot: one_shot_state,
                app_handle: Some(app.handle().clone()),
                token,
            };
            let server_app_dir = app_dir.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) =
                    http_server::bind_and_serve(server_ctx, &server_app_dir).await
                {
                    eprintln!("agent-sweet-home: HTTP API stopped with error: {e}");
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            settings::get_settings,
            settings::save_settings,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

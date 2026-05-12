use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, State};

// 15 minutes — sessions whose last_output_at is older than this are reported
// as `frozen` to the UI / API. The PTY itself is unaffected.
pub const FROZEN_AFTER_SECS: u64 = 15 * 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub id: String,
    pub repo_id: i64,
    pub repo_name: String,
    pub cwd: String,
    pub command: Vec<String>,
    pub started_at: u64,
    pub last_output_at: u64,
    pub uptime_secs: u64,
    pub frozen: bool,
    pub exit_code: Option<i32>,
}

struct LiveSession {
    info: SessionInfo,
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    last_output_at: Arc<AtomicU64>,
    exit_code: Arc<Mutex<Option<i32>>>,
}

#[derive(Default, Clone)]
pub struct Registry {
    sessions: Arc<Mutex<HashMap<String, LiveSession>>>,
}

impl Registry {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn snapshot_all_public(&self, filter_repo_id: Option<i64>) -> Vec<SessionInfo> {
        self.snapshot_all(filter_repo_id)
    }

    fn snapshot_all(&self, filter_repo_id: Option<i64>) -> Vec<SessionInfo> {
        let now = unix_now();
        let map = match self.sessions.lock() {
            Ok(g) => g,
            Err(_) => return vec![],
        };
        map.values()
            .filter(|s| filter_repo_id.map_or(true, |id| s.info.repo_id == id))
            .map(|s| build_info(&s.info, &s.last_output_at, &s.exit_code, now))
            .collect()
    }

    fn snapshot_one(&self, id: &str) -> Option<SessionInfo> {
        let now = unix_now();
        let map = self.sessions.lock().ok()?;
        let s = map.get(id)?;
        Some(build_info(&s.info, &s.last_output_at, &s.exit_code, now))
    }
}

fn build_info(
    base: &SessionInfo,
    last: &AtomicU64,
    exit: &Mutex<Option<i32>>,
    now: u64,
) -> SessionInfo {
    let last_output_at = last.load(Ordering::Relaxed);
    let exit_code = exit.lock().ok().and_then(|g| *g);
    SessionInfo {
        id: base.id.clone(),
        repo_id: base.repo_id,
        repo_name: base.repo_name.clone(),
        cwd: base.cwd.clone(),
        command: base.command.clone(),
        started_at: base.started_at,
        last_output_at,
        uptime_secs: now.saturating_sub(base.started_at),
        frozen: now.saturating_sub(last_output_at) >= FROZEN_AFTER_SECS,
        exit_code,
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn make_session_id(repo_name: &str) -> String {
    let now = unix_now();
    let uuid = uuid::Uuid::new_v4().simple().to_string();
    let short = &uuid[..8];
    format!("{}-{}-{}", sanitize(repo_name), now, short)
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

fn default_command() -> Vec<String> {
    vec![
        "claude".to_string(),
        "--dangerously-skip-permissions".to_string(),
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PtyCreateArgs {
    pub repo_id: i64,
    pub repo_name: String,
    pub cwd: String,
    #[serde(default)]
    pub command: Option<Vec<String>>,
    #[serde(default)]
    pub cols: Option<u16>,
    #[serde(default)]
    pub rows: Option<u16>,
}

#[tauri::command]
pub fn pty_create(
    app: AppHandle,
    registry: State<'_, Registry>,
    args: PtyCreateArgs,
) -> Result<SessionInfo, String> {
    let cwd_buf = expand_tilde(&args.cwd);
    if !cwd_buf.exists() {
        return Err(format!("cwd does not exist: {}", cwd_buf.display()));
    }
    let command = args.command.unwrap_or_else(default_command);
    if command.is_empty() {
        return Err("command must not be empty".into());
    }
    let cols = args.cols.unwrap_or(120);
    let rows = args.rows.unwrap_or(32);

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("openpty failed: {e}"))?;

    let mut cmd = CommandBuilder::new(&command[0]);
    for a in &command[1..] {
        cmd.arg(a);
    }
    cmd.cwd(&cwd_buf);
    if let Some(home) = std::env::var_os("HOME") {
        cmd.env("HOME", home);
    }
    // Augment PATH so GUI-launched app can find binaries installed by
    // Homebrew (~/.local/bin for Claude Code, /opt/homebrew/bin for brew).
    {
        let home = std::env::var("HOME").unwrap_or_default();
        let extra = format!(
            "{}/.local/bin:/opt/homebrew/bin:/usr/local/bin",
            home
        );
        let path = match std::env::var("PATH") {
            Ok(p) if !p.is_empty() => format!("{}:{}", extra, p),
            _ => extra,
        };
        cmd.env("PATH", path);
    }
    cmd.env("TERM", "xterm-256color");

    let mut child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("spawn failed: {e}"))?;
    drop(pair.slave);

    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("reader clone failed: {e}"))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("writer take failed: {e}"))?;

    let id = make_session_id(&args.repo_name);
    let started_at = unix_now();
    let info = SessionInfo {
        id: id.clone(),
        repo_id: args.repo_id,
        repo_name: args.repo_name.clone(),
        cwd: cwd_buf.to_string_lossy().to_string(),
        command: command.clone(),
        started_at,
        last_output_at: started_at,
        uptime_secs: 0,
        frozen: false,
        exit_code: None,
    };

    let last_output_at = Arc::new(AtomicU64::new(started_at));
    let exit_code = Arc::new(Mutex::new(None));

    let session = LiveSession {
        info: info.clone(),
        master: Arc::new(Mutex::new(pair.master)),
        writer: Arc::new(Mutex::new(writer)),
        last_output_at: last_output_at.clone(),
        exit_code: exit_code.clone(),
    };

    {
        let mut map = registry
            .sessions
            .lock()
            .map_err(|_| "registry poisoned".to_string())?;
        map.insert(id.clone(), session);
    }

    let app_for_reader = app.clone();
    let id_for_reader = id.clone();
    let last_output_for_reader = last_output_at.clone();
    let exit_for_reader = exit_code.clone();
    let registry_for_reader: Registry = (*registry).clone();

    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    last_output_for_reader.store(unix_now(), Ordering::Relaxed);
                    let payload = B64.encode(&buf[..n]);
                    let _ = app_for_reader.emit(&format!("pty:output:{id_for_reader}"), payload);
                }
                Err(_) => break,
            }
        }
        let code = match child.wait() {
            Ok(status) => status.exit_code() as i32,
            Err(_) => -1,
        };
        if let Ok(mut g) = exit_for_reader.lock() {
            *g = Some(code);
        }
        let _ = app_for_reader.emit(&format!("pty:exit:{id_for_reader}"), code);
        // Drop from registry once finished. UI listens to the exit event and
        // can decide whether to keep the tile around or unmount.
        if let Ok(mut map) = registry_for_reader.sessions.lock() {
            map.remove(&id_for_reader);
        }
    });

    Ok(info)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PtyWriteArgs {
    pub id: String,
    /// base64-encoded raw bytes to send to the PTY (xterm.js onData payload).
    pub data: String,
}

#[tauri::command]
pub fn pty_write(registry: State<'_, Registry>, args: PtyWriteArgs) -> Result<(), String> {
    let bytes = B64
        .decode(args.data.as_bytes())
        .map_err(|e| format!("invalid base64: {e}"))?;
    let writer = {
        let map = registry
            .sessions
            .lock()
            .map_err(|_| "registry poisoned".to_string())?;
        map.get(&args.id)
            .map(|s| s.writer.clone())
            .ok_or_else(|| format!("unknown session: {}", args.id))?
    };
    let mut w = writer.lock().map_err(|_| "writer poisoned".to_string())?;
    w.write_all(&bytes).map_err(|e| format!("write: {e}"))?;
    w.flush().map_err(|e| format!("flush: {e}"))?;
    Ok(())
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PtyResizeArgs {
    pub id: String,
    pub cols: u16,
    pub rows: u16,
}

#[tauri::command]
pub fn pty_resize(registry: State<'_, Registry>, args: PtyResizeArgs) -> Result<(), String> {
    let master = {
        let map = registry
            .sessions
            .lock()
            .map_err(|_| "registry poisoned".to_string())?;
        map.get(&args.id)
            .map(|s| s.master.clone())
            .ok_or_else(|| format!("unknown session: {}", args.id))?
    };
    let m = master.lock().map_err(|_| "master poisoned".to_string())?;
    m.resize(PtySize {
        rows: args.rows,
        cols: args.cols,
        pixel_width: 0,
        pixel_height: 0,
    })
    .map_err(|e| format!("resize: {e}"))?;
    Ok(())
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PtyKillArgs {
    pub id: String,
}

#[tauri::command]
pub fn pty_kill(registry: State<'_, Registry>, args: PtyKillArgs) -> Result<(), String> {
    let removed = {
        let mut map = registry
            .sessions
            .lock()
            .map_err(|_| "registry poisoned".to_string())?;
        map.remove(&args.id)
    };
    if let Some(s) = removed {
        // Dropping the master closes the PTY; the child receives SIGHUP and
        // the reader thread exits cleanly. We don't have a portable handle
        // to send an explicit SIGTERM through portable-pty, but the SIGHUP
        // is enough for `claude` and a regular shell.
        drop(s);
        Ok(())
    } else {
        Err(format!("unknown session: {}", args.id))
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PtyListArgs {
    #[serde(default)]
    pub repo_id: Option<i64>,
}

#[tauri::command]
pub fn pty_list(registry: State<'_, Registry>, args: PtyListArgs) -> Vec<SessionInfo> {
    registry.snapshot_all(args.repo_id)
}

#[tauri::command]
pub fn pty_get(registry: State<'_, Registry>, id: String) -> Option<SessionInfo> {
    registry.snapshot_one(&id)
}

fn expand_tilde(p: &str) -> PathBuf {
    if let Some(stripped) = p.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(stripped);
        }
    }
    if p == "~" {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home);
        }
    }
    PathBuf::from(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_id_has_expected_shape() {
        let id = make_session_id("agent/sweet home");
        // Only ascii alnum, '-', '_'.
        assert!(id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        let parts: Vec<&str> = id.rsplitn(3, '-').collect();
        // rsplitn yields [uuid, ts, repo] — uuid is the last "-" segment.
        assert_eq!(parts[0].len(), 8);
        assert!(parts[1].parse::<u64>().is_ok());
    }

    #[test]
    fn sanitize_strips_unsafe_chars() {
        assert_eq!(sanitize("repo/name with space"), "repo_name_with_space");
        assert_eq!(sanitize("agent-sweet_home"), "agent-sweet_home");
    }

    #[test]
    fn default_command_targets_claude_with_skip_permissions() {
        let cmd = default_command();
        assert_eq!(cmd[0], "claude");
        assert!(cmd.iter().any(|a| a == "--dangerously-skip-permissions"));
    }

    #[test]
    fn build_info_marks_frozen_after_threshold() {
        let base = SessionInfo {
            id: "x".into(),
            repo_id: 0,
            repo_name: "r".into(),
            cwd: "/".into(),
            command: vec!["sh".into()],
            started_at: 1000,
            last_output_at: 1000,
            uptime_secs: 0,
            frozen: false,
            exit_code: None,
        };
        let last = AtomicU64::new(1000);
        let exit = Mutex::new(None);
        let now = 1000 + FROZEN_AFTER_SECS + 1;
        let info = build_info(&base, &last, &exit, now);
        assert!(info.frozen);
        assert_eq!(info.uptime_secs, FROZEN_AFTER_SECS + 1);
    }

    #[test]
    fn build_info_is_not_frozen_when_recent() {
        let base = SessionInfo {
            id: "x".into(),
            repo_id: 0,
            repo_name: "r".into(),
            cwd: "/".into(),
            command: vec!["sh".into()],
            started_at: 1000,
            last_output_at: 2000,
            uptime_secs: 0,
            frozen: false,
            exit_code: None,
        };
        let last = AtomicU64::new(2000);
        let exit = Mutex::new(None);
        let info = build_info(&base, &last, &exit, 2100);
        assert!(!info.frozen);
        assert_eq!(info.uptime_secs, 1100);
    }

    #[test]
    fn registry_filters_by_repo_id() {
        let registry = Registry::new();
        let now = unix_now();
        let make = |repo_id: i64, name: &str| LiveSession {
            info: SessionInfo {
                id: format!("{name}-{now}-aaaaaaaa"),
                repo_id,
                repo_name: name.into(),
                cwd: "/".into(),
                command: vec!["sh".into()],
                started_at: now,
                last_output_at: now,
                uptime_secs: 0,
                frozen: false,
                exit_code: None,
            },
            // We never read these in the snapshot path so we plug in cheap
            // sentinels using a closed pipe to satisfy the type.
            master: Arc::new(Mutex::new(test_dummies::dummy_master())),
            writer: Arc::new(Mutex::new(Box::new(std::io::sink()))),
            last_output_at: Arc::new(AtomicU64::new(now)),
            exit_code: Arc::new(Mutex::new(None)),
        };
        {
            let mut map = registry.sessions.lock().unwrap();
            let s1 = make(1, "alpha");
            let s2 = make(2, "beta");
            map.insert(s1.info.id.clone(), s1);
            map.insert(s2.info.id.clone(), s2);
        }
        let only_one = registry.snapshot_all(Some(1));
        assert_eq!(only_one.len(), 1);
        assert_eq!(only_one[0].repo_name, "alpha");
        assert_eq!(registry.snapshot_all(None).len(), 2);
    }

    mod test_dummies {
        use portable_pty::{native_pty_system, MasterPty, PtySize};
        pub fn dummy_master() -> Box<dyn MasterPty + Send> {
            // openpty without spawning a child is cheap and disposable.
            let pair = native_pty_system()
                .openpty(PtySize {
                    rows: 24,
                    cols: 80,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .expect("openpty");
            // Drop the slave so the master is otherwise inert.
            drop(pair.slave);
            pair.master
        }
    }
}

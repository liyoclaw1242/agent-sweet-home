# Agent Sweet Home

Tauri 2 + React 19 + TypeScript 桌面應用，把 GitHub repo 列表、open issues / PRs 與本機 clone 狀態集中在一個視窗，並開放 localhost HTTP API 供外部 CLI / agent 查詢。

## 主要功能

### UI

- **Header** — 應用標題 + 右上齒輪鈕（Settings）
- **Sidebar** — 從 GitHub 拉到的 repo 列表，可點選；含 loading / error / 重新整理鈕
- **Tabs**（`Home` / `Persistent (n)` / `One-Shot (n)` / `Cron (n)`） — 每個 repo 各自記憶 active tab，切 repo 不互相影響；未選 repo 時 disabled
- **Settings 對話框** — 三欄位輸入：GitHub 帳號、Personal Access Token、預設本機路徑（例 `~/Projects`，留空時自動 fallback 到 `~/Projects`）；存入 SQLite，重開仍在
- **Home 分頁** — 自動 pre-check 本機路徑 + 拉 issues/PRs：
  - Repo 名 / 預設 branch / 本機路徑（不存在或非 git 倉時顯示徽章）/ 本機目前 branch（含 `clean` 或 `N changes` 徽章）
  - Open issues 列表（`#number`、title、彩色 label）
  - Open PRs 列表（`#number`、title、draft 徽章）
  - 每 15 分鐘自動刷新 issues/PRs；切離 Home 分頁或卸載元件時自動清計時器
  - 手動 Refresh 按鈕一鍵重抓
- **Persistent 分頁** — 在面板裡開真實 PTY 終端機：
  - 預設指令 `claude --dangerously-skip-permissions`，cwd 為該 repo 的本機目錄
  - Grid 佈局 `auto-fit minmax(420px, 1fr)`，會隨終端機數量重排；永遠保留一個虛線 `+` tile 用來開新終端機
  - 每個 tile header 顯示 `${repo}-${unix-secs}-${uuid8}` session id、即時運行時長（秒/分/時自動換單位）、`frozen` 徽章（>15 分鐘無輸出）、`exited <code>` 徽章；右側 `×` 關閉
  - 終端機 IO 走 xterm.js + portable-pty：使用者輸入 → `pty_write`、PTY 輸出 → Tauri event `pty:output:{id}`（base64）→ 寫回 xterm；`ResizeObserver` 自動 `pty_resize`
  - Sessions 為 process lifetime（in-memory registry，App 重啟即清空）；切走 tab 後重新進入會 `pty_list` 重新 hydrate

### 後端（Rust / Tauri）

- **SQLite 持久化**（`<app_data_dir>/agent-sweet-home.db`）
  - `settings` — 三項：GitHub 帳號、Token、Default Local Path
  - `repos` / `issues` / `prs` — 由 `fetch_*` Tauri commands write-through，HTTP API 直接讀快取，避開 GitHub rate limit
- **GitHub API** — `reqwest` + bearer token；issues 過濾掉 PR、PRs 過濾已 merge
- **本機探查** — 展開 `~`、檢查 `{base}/{repoName}` 是否存在、讀 `.git/HEAD` 取 branch（支援 detached HEAD）、`git status --porcelain` 計 dirty 檔數；Settings 留空時 base path 自動 fallback 到 `~/Projects`
- **PTY Registry** — `portable-pty` 跨平台 PTY，每 session 一個 std::thread 讀 master output；`Mutex<HashMap<id, LiveSession>>` 由 Tauri `app.manage()` 持有；Tauri commands：`pty_create` / `pty_write` / `pty_resize` / `pty_kill` / `pty_list` / `pty_get`；events：`pty:output:{id}`（每 chunk 一發）、`pty:exit:{id}`（PTY 結束後同步從 registry 移除）

### 外部 HTTP API（給 CLI / agent / 外部 app）

啟動時會在 `127.0.0.1:0`（OS 選 port）開一個 axum server，並把 port + token 寫入 `<app_data_dir>/server.json`（Unix 上 `chmod 0600`）。

| Endpoint              | 說明                                                                                |
| --------------------- | ----------------------------------------------------------------------------------- |
| `GET /health`         | 不需 token，回 `ok`                                                                  |
| `GET /repos`          | 快取的 repo list                                                                     |
| `GET /repos/{name}`   | 該 repo cached metadata + issues + PRs + live local inspection (git)                 |
| `GET /sessions`       | 目前運行中的 PTY sessions；可用 `?repo=<name>` 或 `?repoId=<id>` 過濾，回每個 session 的 id / repoName / cwd / command / startedAt / lastOutputAt / uptimeSecs / frozen / exitCode |

需帶 `Authorization: Bearer <token>` header（除了 `/health`）。

範例：

```bash
SERVER=$(cat "$HOME/Library/Application Support/com.agentsweethome.app/server.json")
TOKEN=$(echo "$SERVER" | jq -r .token)
PORT=$(echo "$SERVER" | jq -r .port)

curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:$PORT/repos
curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:$PORT/repos/alpha
curl -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:$PORT/sessions?repo=alpha"
```

## 開發指令

```bash
pnpm install        # 安裝依賴
pnpm tauri dev      # 啟動桌面 App（Rust + 前端 HMR）
pnpm dev            # 只開瀏覽器版前端 :1420
pnpm test           # 前端測試 (Vitest)
pnpm test:rust      # Rust 測試 (cargo test)
pnpm test:all       # 兩端一起跑
pnpm build          # 前端正式建置 (tsc + vite build)
pnpm tauri build    # 打包桌面 App 安裝檔
```

## 技術棧

- **前端**：React 19、TypeScript 5.8、Vite 7、純 CSS（含 dark mode）、xterm.js 6 + `@xterm/addon-fit`
- **後端**：Tauri 2.11、Rust 2021 edition
- **資料**：SQLite (`rusqlite` bundled)
- **PTY**：`portable-pty` 0.8（跨平台原生 PTY） + `base64` (IPC bridge)
- **網路**：`reqwest` (rustls)、`axum` 0.8
- **測試**：Vitest 4 + Testing Library + jsdom + 官方 `@tauri-apps/api/mocks`（xterm/FitAddon/ResizeObserver 在 setup.ts 用 `vi.mock` stub）；`cargo test` + `tower::ServiceExt::oneshot` for HTTP

## 資料夾結構

```
src/                      React 前端
├── App.tsx               佈局：Header / Sidebar / Tabs / Main + 各 tab count 收斂
├── components/
│   ├── Header.tsx        標題列 + 齒輪鈕
│   ├── Sidebar.tsx       Repo 列表
│   ├── Tabs.tsx          4 分頁 (Home/Persistent/One-Shot/Cron)
│   ├── HomeView.tsx      Home 分頁內容（pre-check + issues/PRs + 15min 計時器）
│   ├── PersistentView.tsx Persistent 分頁：grid + 開新終端機 + onCountChange 回報
│   ├── TerminalTile.tsx  單個終端機：xterm.js + FitAddon + Tauri PTY IPC/event 橋接
│   └── SettingsDialog.tsx 設定模態框
└── test/setup.ts         Vitest setup + 預設 IPC mock + xterm/ResizeObserver mock

src-tauri/src/            Rust 後端
├── lib.rs                Tauri builder + setup() 啟動 HTTP server + 註冊 PTY Registry
├── db.rs                 Db = Arc<Mutex<Connection>> + migrations
├── settings.rs           get/save_settings commands
├── github.rs             fetch_repos/issues/prs commands (write-through 快取)
├── local_repo.rs         inspect_local_repo command + inspect_at（無設定 fallback `~/Projects`）
├── cache.rs              SQLite 快取讀寫
├── terminal.rs           PTY Registry + pty_create/write/resize/kill/list/get commands
└── http_server.rs        axum router + bearer token 認證 + /sessions 端點
```

## 安全備註

- GitHub token 與 HTTP API token 目前**明文**存於 SQLite / `server.json`；`server.json` 已限為 `0600`，但若要更嚴格建議改用 OS keychain（`tauri-plugin-stronghold` 或 `keyring` crate）
- HTTP server 只綁 `127.0.0.1`，不對外網暴露
- API token 為 process lifetime，每次啟動會重新生成
- Persistent 終端機預設帶 `--dangerously-skip-permissions` 跑 `claude`，等於賦予完整檔案系統 / 命令執行權限——只在你信任的本機 repo 目錄使用，不要把 `/sessions` API 透過 reverse proxy 對外暴露

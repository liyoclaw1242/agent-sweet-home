# Agent Sweet Home

Tauri 2 + React 19 + TypeScript 桌面應用，把 GitHub repo 列表、open issues / PRs、本機 clone 狀態，以及對該 repo 跑的 Claude Code session（互動式 PTY 終端機 + 一次性 `claude -p` 任務）集中在一個視窗，並以宣告式 YAML workflow 引擎自動 dispatch GitHub issues 給 AI agent。同時開放 localhost HTTP API 供外部 CLI / agent 查詢與操作。

## 主要功能

### UI

- **Header** — 應用標題 + 右上齒輪鈕（Settings）
- **Sidebar** — 從 GitHub 拉到的 repo 列表，可點選；含 loading / error / 重新整理鈕
- **Tabs**（`Home` / `Persistent (n)` / `One-Shot (n)` / `Cron (n)` / `Workflow`） — 每個 repo 各自記憶 active tab，切 repo 不互相影響；未選 repo 時 disabled
  - `Persistent (n)` 顯示目前運行中的終端機數量
  - `One-Shot (n)` 顯示目前 status=`running` 的 one-shot 任務數量
- **Settings 對話框** — 三欄位輸入：GitHub 帳號、Personal Access Token、預設本機路徑（例 `~/Projects`，留空時 backend 自動 fallback 到 `~/Projects`）；存入 SQLite，重開仍在
- **Workflow 分頁** — 宣告式 YAML workflow 引擎的狀態面板：
  - YAML 路徑設定輸入框（儲存到 SQLite，重啟後生效）；優先級 `WORKFLOW_FILE` env > 儲存路徑 > `app_data_dir/workflow.yaml`
  - 顯示目前生效路徑 + 載入狀態（綠點=loaded / 紅點=parse error / 灰點=missing）
  - YAML 內容預覽（`<pre>`）
  - Workflow 啟動後每次 GitHub issue 符合 dispatch rule 就自動 spawn `claude -p`；spawn 的 run 會出現在對應 repo 的 One-Shot 分頁
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
- **One-Shot 分頁** — 跑 `claude -p`（headless）任務並串流 log：
  - 左欄 run list（id / 截短 prompt / status badge / 起始時間 / 累計成本 USD），右欄 log panel（NDJSON 一行一個事件，stderr 紅色高亮）
  - `+ New run` 開 modal 填參數：基本（Prompt / Model / Output format / Skip permissions）+ 三個折疊區（權限 & 工具 & 預算 / System prompt & Add dir / Session & MCP & 逃生口）
  - 預設 `--output-format stream-json --verbose`；`--include-partial-messages` 只在 stream-json 才能勾
  - 啟動後即時 listen `oneshot:line:{id}` 事件，看到 `{"type":"result","total_cost_usd":...}` 把成本回填 DB；結束 emit `oneshot:exit:{id}` 並把 status 標為 `completed`/`failed`
  - 跑中可 `Kill`（送 SIGKILL + 寫一筆 `[killed by user]` 到 log）；已結束可 `Delete`（從 DB cascade 刪除 run + log lines）
  - Run 與 log lines 都寫進 SQLite，重開 App 仍可查詢歷史

### 後端（Rust / Tauri）

- **PATH augmentation** — GUI 啟動的 app 繼承的 `PATH` 通常缺少 Homebrew / user local bins；`one_shot`、`terminal`、`workflow/command` 三處統一在執行子程序前 prepend `~/.local/bin:/opt/homebrew/bin:/usr/local/bin`，確保 `claude`、`gh` 等工具可找到
- **SQLite 持久化**（`<app_data_dir>/agent-sweet-home.db`，migration 冪等）
  - `settings` — 四項：GitHub 帳號、Token、Default Local Path、workflow YAML 路徑
  - `repos` / `issues` / `prs` — 由 `fetch_*` Tauri commands write-through，HTTP API 直接讀快取，避開 GitHub rate limit
  - `one_shot_runs` / `one_shot_log_lines` — One-Shot 任務 metadata 與每行 stdout/stderr，`ON DELETE CASCADE` 一起刪
- **GitHub API** — `reqwest` + bearer token；issues 過濾掉 PR、PRs 過濾已 merge
- **本機探查** — 展開 `~`、檢查 `{base}/{repoName}` 是否存在、讀 `.git/HEAD` 取 branch（支援 detached HEAD）、`git status --porcelain` 計 dirty 檔數；Settings 留空時 base path 自動 fallback 到 `~/Projects`
- **PTY Registry**（Persistent 用） — `portable-pty` 跨平台 PTY，每 session 一個 std::thread 讀 master output；`Mutex<HashMap<id, LiveSession>>` 由 Tauri `app.manage()` 持有；events：`pty:output:{id}`（每 chunk 一發 base64）、`pty:exit:{id}`（PTY 結束後同步從 registry 移除）
- **One-Shot Runner** — `Command::spawn` + stdin=null + stdout/stderr piped；兩條 reader thread 逐行 `read_line` → 寫 `one_shot_log_lines` → emit `oneshot:line:{id}`；watcher thread `wait()` 完 emit `oneshot:exit:{id}` 並把 status 標 completed/failed；`OneShotState` 持 `HashMap<id, Arc<Mutex<Child>>>` 提供 kill 路徑

### Tauri Commands（前端 IPC）

| Command                           | 說明                                                            |
| --------------------------------- | --------------------------------------------------------------- |
| `get_settings` / `save_settings`  | 讀 / 寫 SQLite 中的設定（GitHub 帳號、Token、本機路徑）          |
| `save_workflow_path`              | 單獨寫入 `workflow_path` 設定；重啟後 workflow 引擎使用新路徑     |
| `fetch_repos`                     | 拉 GitHub repo 並 write-through 快取                            |
| `fetch_issues` / `fetch_prs`      | 同上，鎖一個 `repoFullName`                                     |
| `inspect_local_repo`              | 對 `{base}/{repoName}` 做本機探查（存在性 / git / branch / dirty） |
| `pty_create` / `pty_write` / `pty_resize` / `pty_kill` / `pty_list` / `pty_get` | Persistent 終端機 PTY 控制 |
| `one_shot_start`                  | 啟動 `claude -p` 子程序，建立 run row，回 `RunInfo`             |
| `one_shot_list`                   | 列出 runs（可帶 `repoId` / `status` 過濾）                       |
| `one_shot_get`                    | 單筆 metadata                                                    |
| `one_shot_log`                    | 取 log lines；`sinceSeq` 給上次拿到的最大 seq 做增量 polling     |
| `one_shot_kill`                   | 跑中 → kill；已結束 → 從 DB 刪掉 run + log lines                 |
| `workflow_status`                 | 回目前 workflow 載入狀態 + YAML 內容（供 WorkflowView 顯示）     |

### Tauri Events（後端 → 前端）

| Event                  | Payload                                            |
| ---------------------- | -------------------------------------------------- |
| `pty:output:{id}`      | base64-encoded PTY stdout chunk                    |
| `pty:exit:{id}`        | exit code (number)                                 |
| `oneshot:line:{id}`    | `{ runId, seq, ts, stream: "stdout"\|"stderr", text }` |
| `oneshot:exit:{id}`    | `{ exitCode, status: "completed"\|"failed" }`      |

### 外部 HTTP API（給 CLI / agent / 外部 app）

啟動時會在 `127.0.0.1:0`（OS 選 port）開一個 axum server，並把 port + token 寫入 `<app_data_dir>/server.json`（Unix 上 `chmod 0600`）。所有端點（除了 `/health`）需帶 `Authorization: Bearer <token>` header。

| Method | Endpoint                  | 說明                                                                |
| ------ | ------------------------- | ------------------------------------------------------------------- |
| GET    | `/health`                 | 不需 token，回 `ok`                                                 |
| GET    | `/repos`                  | 快取的 repo list                                                    |
| GET    | `/repos/{name}`           | 該 repo cached metadata + issues + PRs + live local inspection      |
| GET    | `/sessions`               | 運行中的 PTY sessions；`?repo=<name>` 或 `?repoId=<id>` 過濾         |
| GET    | `/one-shot`               | One-shot run history；`?repo=<name>` / `?repoId=<id>` / `?status=running\|completed\|failed\|killed` 過濾，依 `started_at DESC` 排序 |
| POST   | `/one-shot`               | 啟動新的 `claude -p` 任務（body 同 `one_shot_start` 的 args，camelCase JSON），回 `RunInfo` |
| GET    | `/one-shot/{id}`          | 單筆 metadata（`RunInfo`，含 argv、status、cost…）                  |
| GET    | `/one-shot/{id}/log`      | 該 run 的 log lines；`?since=<seq>` 增量 polling、`?limit=<n>`（預設 1000） |
| DELETE | `/one-shot/{id}`          | 跑中 → 202 Accepted（送 SIGKILL）；已結束 → 204 No Content（DB 刪掉） |
| GET    | `/workflow`               | 目前 workflow 載入狀態（`path`、`exists`、`loaded`、`error`）        |
| POST   | `/workflow/path`          | 設定 workflow YAML 路徑（body: `{ "path": "..." }`）；重啟後生效     |

#### `RunInfo`（GET /one-shot 等回傳）

```json
{
  "id": "alpha-1716489600-aabbccdd",
  "repoId": 1,
  "repoName": "alpha",
  "cwd": "/Users/me/Projects/alpha",
  "prompt": "Refactor the auth flow",
  "argv": ["claude","-p","--output-format","stream-json","--verbose","Refactor the auth flow"],
  "status": "running",
  "startedAt": 1716489600,
  "endedAt": null,
  "exitCode": null,
  "totalCostUsd": null,
  "outputFormat": "stream-json"
}
```

#### `LogLine`（GET /one-shot/{id}/log 回傳的元素）

```json
{
  "runId": "alpha-1716489600-aabbccdd",
  "seq": 0,
  "ts": 1716489601000,
  "stream": "stdout",
  "text": "{\"type\":\"system\",\"subtype\":\"init\",...}"
}
```

#### `POST /one-shot` 完整參數

所有欄位皆為 camelCase JSON。`repoId`/`repoName`/`cwd` 必填；其他都有合理預設。`extraArgs` 是逃生口——任何沒被一級欄位覆蓋的 CLI flag 直接列在這裡，會原樣 append 到 argv 尾端、prompt 之前。

| 欄位                     | 型別                | 對應 `claude` flag                       | 預設                |
| ------------------------ | ------------------- | ---------------------------------------- | ------------------- |
| `repoId`                 | number (required)   | —                                        | —                   |
| `repoName`               | string (required)   | —                                        | —                   |
| `cwd`                    | string (required)   | `Command::current_dir`，支援 `~/`        | —                   |
| `prompt`                 | string              | positional                                | `""`（搭 continue/resume 時可空） |
| `model`                  | string              | `--model`                                | inherit             |
| `outputFormat`           | `"text"\|"json"\|"stream-json"` | `--output-format`                | `"stream-json"`     |
| `permissionMode`         | enum                | `--permission-mode`                      | inherit             |
| `skipPermissions`        | bool                | `--dangerously-skip-permissions`         | `false`             |
| `effort`                 | `"low"\|"medium"\|"high"\|"xhigh"\|"max"` | `--effort`                | inherit             |
| `verbose`                | bool                | `--verbose`                              | `true`              |
| `includePartialMessages` | bool                | `--include-partial-messages`（自動跳過非 stream-json）| `false` |
| `systemPrompt`           | string              | `--system-prompt`                        | —                   |
| `appendSystemPrompt`     | string              | `--append-system-prompt`                 | —                   |
| `addDir`                 | string[]            | `--add-dir <d>`（重複）                  | `[]`                |
| `allowedTools`           | string[]            | `--allowedTools "<comma-joined>"`        | `[]`                |
| `disallowedTools`        | string[]            | `--disallowedTools "..."`                | `[]`                |
| `tools`                  | string              | `--tools <s>`                            | —                   |
| `agent`                  | string              | `--agent`                                | —                   |
| `maxBudgetUsd`           | number              | `--max-budget-usd`                       | —                   |
| `mcpConfig`              | string[]            | `--mcp-config <p>`（重複）               | `[]`                |
| `strictMcpConfig`        | bool                | `--strict-mcp-config`                    | `false`             |
| `resume`                 | string              | `--resume <uuid>`                        | —                   |
| `continueLast`           | bool                | `--continue`                             | `false`             |
| `forkSession`            | bool                | `--fork-session`                         | `false`             |
| `name`                   | string              | `--name`                                 | —                   |
| `extraArgs`              | string[]            | passthrough                              | `[]`                |

### 外部 HTTP API 範例

```bash
SERVER=$(cat "$HOME/Library/Application Support/com.agentsweethome.app/server.json")
TOKEN=$(jq -r .token <<<"$SERVER")
PORT=$(jq -r .port <<<"$SERVER")
H="Authorization: Bearer $TOKEN"

# 1. 探查
curl -H "$H" http://127.0.0.1:$PORT/repos
curl -H "$H" http://127.0.0.1:$PORT/repos/alpha

# 2. 看跑中的 PTY 終端機（Persistent tab）
curl -H "$H" "http://127.0.0.1:$PORT/sessions?repo=alpha"

# 3. 啟動一個 one-shot 任務
RUN=$(curl -s -H "$H" -H 'content-type: application/json' \
  -d '{
    "repoId": 1,
    "repoName": "alpha",
    "cwd": "~/Projects/alpha",
    "prompt": "找出所有 TODO 並列成清單",
    "model": "sonnet",
    "skipPermissions": true,
    "verbose": true
  }' \
  http://127.0.0.1:$PORT/one-shot)
RUN_ID=$(jq -r .id <<<"$RUN")
echo "Started $RUN_ID"

# 4. 增量輪詢 log（用 sinceSeq 拿新行）
SEQ=-1
while :; do
  LINES=$(curl -s -H "$H" "http://127.0.0.1:$PORT/one-shot/$RUN_ID/log?since=$SEQ&limit=200")
  echo "$LINES" | jq -r '.[] | "[\(.stream)] \(.text)"'
  SEQ=$(jq 'if length>0 then map(.seq) | max else '"$SEQ"' end' <<<"$LINES")
  STATUS=$(curl -s -H "$H" "http://127.0.0.1:$PORT/one-shot/$RUN_ID" | jq -r .status)
  [ "$STATUS" = "running" ] || break
  sleep 1
done

# 5. 列歷史 / 過濾還在跑的
curl -H "$H" "http://127.0.0.1:$PORT/one-shot?repo=alpha&status=running"

# 6. Kill 或刪掉一個 run
curl -X DELETE -H "$H" http://127.0.0.1:$PORT/one-shot/$RUN_ID

# 7. 查 workflow 載入狀態
curl -H "$H" http://127.0.0.1:$PORT/workflow

# 8. 設定 workflow YAML 路徑（重啟後生效）
curl -s -H "$H" -H 'content-type: application/json' \
  -d '{"path":"/abs/path/to/workflow.yaml"}' \
  http://127.0.0.1:$PORT/workflow/path
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
- **PTY / 子程序**：`portable-pty` 0.8（Persistent 終端機）+ `std::process::Command`（One-Shot）+ `base64` (PTY → IPC bridge)
- **網路**：`reqwest` (rustls)、`axum` 0.8（含 `extract::Query`、method routing）
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
│   ├── OneShotView.tsx   One-Shot 分頁：runs 列表 + log panel + listen oneshot:line:{id}
│   ├── OneShotModal.tsx  新 run 表單（基本 + 三個折疊區）
│   └── SettingsDialog.tsx 設定模態框
└── test/setup.ts         Vitest setup + 預設 IPC mock + xterm/ResizeObserver mock

src-tauri/src/            Rust 後端
├── lib.rs                Tauri builder + setup() 啟動 HTTP server + 註冊 PTY/OneShot state
├── db.rs                 Db = Arc<Mutex<Connection>> + migrations（含 one_shot_*）
├── settings.rs           get/save_settings commands
├── github.rs             fetch_repos/issues/prs commands (write-through 快取)
├── local_repo.rs         inspect_local_repo command + inspect_at（無設定 fallback `~/Projects`）
├── cache.rs              SQLite 快取讀寫
├── terminal.rs           PTY Registry + pty_create/write/resize/kill/list/get commands
├── one_shot.rs           One-Shot Runner + build_argv + start_run + one_shot_* commands
├── http_server.rs        axum router + bearer token 認證 + /sessions + /one-shot/*
└── workflow/             宣告式 YAML workflow 引擎（完整 runtime）
    ├── mod.rs            load() + start() + workflow_status Tauri command
    ├── spec.rs           全部 serde 結構（Workflow / EntryConfig / actions / control flow）
    ├── dispatch.rs       純函式 dispatch(IssueSnapshot, [DispatchRule]) -> Directive
    ├── expr.rs           minijinja 包裝層（has_label / matches_label / 模板 render）
    ├── runtime.rs        WorkflowRuntime — dispatch_one() 完整 pipeline（worktree → spawn → on_result）
    ├── spawn.rs          build_run_args + run_spawn；橋接 one_shot::start_run
    ├── result.rs         on_result + degrade + unblock_pass
    ├── command.rs        {var} template render + run_capture / run_capture_full（含 PATH augment）
    ├── worktree.rs       git worktree allocate / cleanup（write-isolation 用）
    └── entry/            poll / webhook / manual 三種 entry mode（IssueSource trait）
```

## Workflow 引擎

宣告式 YAML 引擎，在 app 啟動時自動載入 `workflow.yaml`，並以 poll / webhook / manual 三種 entry mode 持續掃描 GitHub issues；符合 `dispatch:` rule 的 issue 會被 `spawn_fresh` 到對應 role 的 `claude -p` 子程序。

### 執行流程

```
Entry (poll/webhook/manual)
  → dispatch()          — 找第一條匹配 rule，取得 Directive
  → pre_spawn checks    — abort / reroute 判斷
  → worktree allocate   — 需要 write isolation 時 git worktree carve
  → start_run()         — 以 one_shot 模組啟動 claude -p
  → wait_until_done()   — 輪詢 DB row 直到 status ≠ "running"
  → on_result / degrade — 解析結構化 JSON output，執行 GitHub 副作用
  → worktree cleanup
```

### YAML 路徑優先級

1. `WORKFLOW_FILE` 環境變數（最高）
2. Settings → Workflow 分頁儲存的路徑（SQLite）
3. `<app_data_dir>/workflow.yaml`（預設 fallback）

### Spawn 與 One-Shot 整合

Workflow 透過 `one_shot::start_run` 啟動 agent，run 記錄寫入同一張 `one_shot_runs` table，並正確綁定 `repo_id`（以 `full_name` 查找）。因此 workflow 自動觸發的 `claude -p` run 會出現在 UI 的 **One-Shot 分頁**，可查 log、kill、刪除，一如手動啟動的 run。

完整參考文件：[`WORKFLOW.md`](./WORKFLOW.md) — 為 AI agent 閱讀優化（schema 對照表、predicate atoms / actions / control flow 全表、gotchas、test fixtures 索引）。

## 安全備註

- GitHub token 與 HTTP API token 目前**明文**存於 SQLite / `server.json`；`server.json` 已限為 `0600`，但若要更嚴格建議改用 OS keychain（`tauri-plugin-stronghold` 或 `keyring` crate）
- HTTP server 只綁 `127.0.0.1`，不對外網暴露
- API token 為 process lifetime，每次啟動會重新生成
- Persistent 終端機預設帶 `--dangerously-skip-permissions` 跑 `claude`，等於賦予完整檔案系統 / 命令執行權限——只在你信任的本機 repo 目錄使用，不要把 `/sessions` API 透過 reverse proxy 對外暴露
- One-Shot 的 `POST /one-shot` 同樣會在本機跑 `claude` 子程序，並可透過 `skipPermissions` / `permissionMode: "bypassPermissions"` / `extraArgs` 拿到等同的執行權限；對外暴露這條 API 等於把整台機器交出去
- One-Shot 的完整 prompt、argv、log 全部以明文寫進 SQLite（無加密），含 secret 的 prompt 自行斟酌

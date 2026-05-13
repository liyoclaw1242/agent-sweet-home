# Design System — Agent Sweet Home

> An anodized aluminum dispatch station for one operator coordinating a fleet of autonomous workers. Cold, weighted surfaces. Every light means something. The cost meter is the loudest object on screen. It assumes you are competent.

## Product Context

- **What this is:** Tauri 2 + React 19 desktop app. A workstation where one developer commands many Claude Code agents (persistent xterm.js terminals + one-shot `claude -p` headless runs + cron + workflow YAML) against many GitHub repos. Streams logs, tracks cost in real time, exposes a localhost HTTP API for external agents.
- **Who it's for:** Solo operators running AI fleets. People who have 5 terminals streaming and 3 one-shots burning real money simultaneously, and need to know WHICH ones at a glance.
- **Space:** AI agent tooling / operator-grade developer tools. Peers: Warp, Cursor, Linear (UI conventions), but the actual ergonomic reference set is Bloomberg Terminal, htop, mission control NOCs, professional DAWs.
- **Project type:** Dense multi-pane desktop app. Dark-first. Operator-grade. Not consumer SaaS.

## Memorable Thing

**Builder's cockpit / control surface — specifically a harbormaster's dispatch station.** When the user opens this app, the 3-second feeling should be *"oh, this respects me."* Not "wow pretty." Not "clever." Respect. The room temperature drops two degrees, the cost meter is already ticking, three repos are visible at once via the manifest strip, the amber pills tell the truth. Nothing is congratulating the user.

The closest emotional analog: opening `htop` for the first time, sitting down at a Bloomberg terminal, or launching Reaper. *"This tool assumes I am competent."*

## Aesthetic Direction

- **Direction:** Industrial / utilitarian with brutalist edges. Anodized aluminum, cold-rolled steel, marine wayfinding.
- **Decoration level:** Minimal. Every decorative element must answer "what telemetry does this carry?" — if the answer is "none," delete it. The status colors are the decoration.
- **Mood:** Cold, weighted, peripherally aware. The dispatch desk at 2am in a working harbor.
- **Anti-pattern guard:** No fake industrial decoration (no rivets, no CRT scanlines, no ASCII chrome). No dashboard-grid metric cards. No equal visual weight across IA siblings — terminals are full-bleed mono, cron is tabular, workflow is sans document.

## Typography

Two faces, no more. **Surgical** use of mono — chrome stays sans, mono carries IDs / paths / durations / costs / logs / xterm.

- **Chrome · UI · Body:** **IBM Plex Sans Condensed** — open-source, real personality (curved R-leg, slab-ish terminals), packs density without compressed-Inter shame. Weights `400 / 500 / 600 / 700`.
- **Numerics · IDs · Paths · Durations · Costs · Logs · Terminal:** **IBM Plex Mono** — same designer as Plex Condensed, identical x-height, native pairing. `font-variant-numeric: tabular-nums` locked on globally. Weights `400 / 500 / 600`.
- **Section headers / labels:** Plex Condensed, `11px`, `text-transform: uppercase`, `letter-spacing: 0.08em`, weight `600`. The only place that reads "designed."

### Loading

Google Fonts via `<link rel="preconnect">` (or self-hosted in production):

```html
<link rel="preconnect" href="https://fonts.googleapis.com" />
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin />
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=IBM+Plex+Sans+Condensed:wght@400;500;600;700&family=IBM+Plex+Mono:wght@400;500;600&display=swap" />
```

### Type Scale

| Token | px / line-height | Weight | Usage |
|---|---|---|---|
| `text-xs` | 10 / 1.2 | 600 | Section headers (uppercase tracked) |
| `text-sm` | 11 / 1.35 | 500 | Pills, meta, weather labels, badge text |
| `text-base` | 12 / 1.4 | 500 | Sidebar items, manifest strip, terminal output |
| `text-md` | 13 / 1.45 | 500 | Body, channel labels |
| `text-lg` | 14 / 1.5 | 500 | Default body, list items, form labels |
| `text-xl` | 16 / 1.4 | 500 | Cost meter, prominent inline numerics |
| `text-2xl` | 18 / 1.3 | 600 | Section H2 |
| `text-3xl` | 22 / 1.2 | 600 | Panel titles |
| `text-display` | 28 / 1.1 | 600 | App-level headings, splash |

## Color

Five surface steps. Three text steps. Five **reserved** signal colors (each means exactly one thing). One non-signal chrome accent. That's it.

### Dark Mode (default — design here lives or dies)

```css
--surface-0:  #0C1014;  /* app base */
--surface-1:  #141921;  /* panel · sidebar */
--surface-2:  #1D2430;  /* raised · hover */
--border-lo:  #2A3340;  /* divider */
--border-hi:  #3D4A5C;  /* important edge */

--text-1:     #E8EAED;  /* primary */
--text-2:     #A7B0BC;  /* secondary */
--text-3:     #6B7785;  /* tertiary · metadata */
```

### Status Signals — RESERVED, never chrome

Each color means exactly one thing. Never use a signal color for branding, buttons, links, focus, or selection. **If a status color shows up, it carries semantic meaning.**

```css
--status-running:  #F5A524;  /* amber · active process, > 0 stdout/stderr in last 15min */
--status-healthy:  #3FB950;  /* completed · exit 0 · clean working tree · success */
--status-failed:   #F85149;  /* error · killed · exit != 0 · stderr-only */
--status-frozen:   #7C8AA0;  /* idle · paused · no output > 15min · desaturated by design */
--status-burn:     #FF6B35;  /* cost meter > 80% of session budget · distinct from failure */
```

### Chrome Accent — NON-signal

```css
--accent: #5BC0EB;  /* harbor cyan · focus rings · selection · Cmd+K active · brand · links */
```

Zero overlap with status semantics. This is the only color that means "interactive surface."

### Light Mode (functional, not hero)

```css
--surface-0:  #F5F2EC;  /* warm bone */
--surface-1:  #ECE7DC;
--surface-2:  #DDD6C6;
--border-lo:  #C9C0AC;
--border-hi:  #A89D86;

--text-1:     #0E1218;  /* ink */
--text-2:     #3A4250;
--text-3:     #6B7785;

--accent:     #0B7FAB;  /* darker harbor cyan for contrast on bone */
```

Status colors are unchanged in light mode. Implement via `:root` defaults + `html[data-theme='light']` overrides on the same CSS custom properties.

## Spacing

```css
--space-2:   2px;
--space-xs:  4px;   /* base unit */
--space-6:   6px;
--space-sm:  8px;
--space-md:  12px;
--space-lg:  16px;
--space-xl:  24px;
--space-2xl: 32px;
--space-3xl: 48px;
```

- **Base unit:** 4px
- **Density:** Compact — operator software, 8-hour usage, tight beats airy
- **Most chrome:** lives at 4 / 8 / 12

## Layout

- **Approach:** Grid-disciplined fixed app shell (Slack-style IA).
- **App shell grid:**
  - Title bar: `28px` (macOS-style traffic-light placeholder)
  - Top bar: `36px` (repo dropdown · Cmd+K · cost meter · settings)
  - **Manifest strip: `28px`** (horizontal pills · every active process across all repos · scrolling)
  - Sidebar: `240px` fixed-but-resizable
  - Right detail panel: `320px` collapsible
  - Main: flex, contains tabs (`32px`) + content + weather (`4px`)
- **Border radius:** `2px` global maximum. `0px` on terminal frame and data tables. No bubble radius anywhere.
- **Borders:** `1px solid var(--border-lo)` for dividers, `1px solid var(--border-hi)` for important edges. Never use shadows for separation — use borders.
- **Density per section type:** Terminals = edge-to-edge mono, no padding. One-Shot = master-detail mixed. Cron = tabular with breathing room. Workflow = generous sans document. Sibling sections in the IA get **different interior density**, not equal weight.

## Signature Layout Moves

These are what makes Agent Sweet Home Agent Sweet Home, not Linear-in-amber.

### 1. The Manifest Strip

A `28px` horizontal band pinned below the top bar showing **every active process across every repo** as scrolling status pills. Each pill: 4px status dot + repo glyph + truncated channel name + live duration in Plex Mono. Click jumps to that process.

**Why:** Slack-style IA assumes you only care about the current channel. Operators of AI fleets need ambient awareness of all repos because runs you started 20 minutes ago in another repo are burning your money right now. The Manifest Strip is the harbormaster's window onto the harbor.

### 2. The Loud Cost Meter

`$0.4732 / $2.00` in Plex Mono tabular figures, fixed in the top bar, ticking in real time as NDJSON streams in. Color-coded against session budget:

- `< 50%`: `var(--accent)` harbor cyan
- `50% – 80%`: `var(--status-running)` amber
- `> 80%`: `var(--status-burn)` `#FF6B35`

**Why:** Bloomberg shows P&L. Agent Sweet Home shows $/session. Same energy. Every other AI dev tool hides cost in a settings panel because consumer SaaS trained designers to hide pricing. Reject that training.

### 3. Status-Colored 4px Left Rail

Every sidebar item gets a `4px` colored left rail showing its live status. Selected item: harbor cyan rail. Running: amber. Frozen: slate. Failed: red. The rail is **`4px`, not `2px`** — it must read from across the room.

**Why:** Turns the sidebar into a vital-signs strip. Peripheral-vision legibility for fleet state.

### 4. Log Weather Sparkline

A `4px`-tall density band at the bottom of each terminal/run pane showing log throughput over the last 60 seconds, colored by stderr/stdout ratio (cyan for stdout-dominated, amber spikes for stderr).

**Why:** Every dev tool shows you *whether* a process is running. None show you its *vitality*. Borrowed from network NOCs (link utilization sparklines).

## Components

### Buttons

- `2px` border-radius. Never round.
- `28px` height standard. `24px` compact.
- **Primary:** `background: var(--accent)`, `color: #0C1014`. Solid, not gradient.
- **Secondary:** `background: var(--surface-2)`, `border: 1px solid var(--border-lo)`. Hover: border becomes `var(--accent)`.
- **Ghost:** transparent. Hover: `background: var(--surface-2)`.
- **Danger:** transparent, `color: var(--status-failed)`, `border: 1px solid rgba(248,81,73,0.3)`.

### Status Badges

```css
.badge {
  display: inline-flex; align-items: center; gap: 4px;
  padding: 1px 6px; border-radius: 2px;
  font-family: var(--font-mono); font-size: 10px;
  text-transform: uppercase; letter-spacing: 0.05em;
}
.badge-running { background: rgba(245,165,36,.12); color: var(--status-running); border: 1px solid rgba(245,165,36,.3); }
.badge-ok      { background: rgba(63,185,80,.12);  color: var(--status-healthy); border: 1px solid rgba(63,185,80,.3);  }
.badge-failed  { background: rgba(248,81,73,.12);  color: var(--status-failed);  border: 1px solid rgba(248,81,73,.3);  }
.badge-frozen  { background: rgba(124,138,160,.12); color: var(--status-frozen); border: 1px solid rgba(124,138,160,.3); }
```

### Form Fields

- Background: `var(--surface-0)`. Border: `1px solid var(--border-lo)`. `2px` radius.
- Focus: border becomes `var(--accent)`. No glow, no shadow.
- Labels: Plex Condensed, `10px`, uppercase, tracked `0.06em`, `var(--text-3)`.

### Tables

- Mono body, sans headers. Zero radius (`0px`).
- Header: uppercase tracked, `var(--text-3)`, `1px solid var(--border-hi)` bottom border.
- Row hover: `var(--surface-2)`.
- Numerics columns: `text-align: right`, tabular-nums.

## Motion

Minimal-functional only. The fleet doesn't shimmer.

```css
--ease-out:    cubic-bezier(0.2, 0.8, 0.2, 1);
--ease-in:     cubic-bezier(0.4, 0, 1, 1);
--ease-in-out: cubic-bezier(0.4, 0, 0.2, 1);

--dur-press: 80ms;
--dur-hover: 120ms;
--dur-panel: 180ms;
--dur-route: 240ms;
```

- **Hover transitions:** `120ms ease-out` on `background-color`, `border-color`, `color` only.
- **Press feedback:** `80ms ease-in` opacity to `0.8`.
- **Panel slide (right detail open/close):** `180ms ease-in-out`.
- **Route changes:** `240ms` max.
- **NO entrance animations.** No fade-in on mount. No staggered list reveals.
- **NO scroll choreography.** No parallax. No scroll-driven anything.
- **Status indicators are static** except the running amber dot, which gets a subtle 6px box-shadow glow at full opacity. No pulse. No animation. Glow alone reads "active."

## Anti-Slop Hard Rules

When generating code or designs against this system, **reject** the following patterns:

1. **No purple or violet anywhere.** Including gradients, accents, hover states, status colors. The category convergence color.
2. **No gradients on UI chrome.** Solid colors only. Borders carry hierarchy, not gradient depth.
3. **No `border-radius > 2px`.** Bubble radius is a SaaS signal; we are not SaaS.
4. **No `Inter`, `Roboto`, `Helvetica`, `Geist`, `Space Grotesk`, or `system-ui` as primary fonts.** Plex Condensed + Plex Mono are the only system fonts.
5. **No mono-only UI.** Mono is surgical — IDs / paths / durations / costs / logs / xterm only. Sidebar labels in sans.
6. **No status color used as chrome.** Amber means "running." It does not mean "click me," "selected," or "primary action."
7. **No 3-column icon grid.** Dashboard-card SaaS pattern. Reject in any new view.
8. **No centered-everything layouts.** Left-aligned, dense, operator-grade.
9. **No drop shadows for elevation.** Use border-color steps (`border-lo` → `border-hi`) to indicate hierarchy.
10. **No fake industrial decoration.** No ASCII borders, CRT scanlines, faux rivets, terminal-style `[████░░░]` ASCII progress bars in chrome. Industrial aesthetics earn their place by being **load-bearing**.
11. **No avatars, no illustrations, no emoji in UI chrome.** Status dots only.
12. **No "Built for X" / "Designed for Y" marketing copy patterns.** This isn't a marketing site.

## Future Considerations (post-v1)

Borrowed from a parallel "Agent Twin" prototype the user generated separately. **Not shipping in v1** because the prototype's overall direction (light cream + chat REPL + bubble radius) conflicts with the harbormaster brief. But three of its moves transplant cleanly into ASH and are queued for a v0.2 evaluation after the cockpit baseline ships.

### FC-1 · Repo card at the top of the sidebar (replaces the top-bar dropdown)

Move the repo switcher out of the top bar and into the first slot of the sidebar as a tall card showing:

- Repo full name + private/public glyph
- Local branch name + ahead/behind count vs. default branch
- Last commit subject + relative time + author
- Today's spend in mono (`$1.47 / $5.00 cap`)

Frees ~180px of top-bar horizontal space (Cmd+K can grow) and packs more information per glance. The dropdown action moves to a small `▾` button inside the card.

**Cost:** sidebar height pressure (card eats ~96px from channel-list area).

### FC-2 · Slash-command pills as One-Shot quick-launch (does NOT replace the modal)

Below the One-Shot run-detail panel, add a row of slash-command pills for saved preset prompts: `/review-pr`, `/fix-lint`, `/update-readme`, `/explain-diff`, etc. One click launches a one-shot with stored parameters — no modal.

Modal stays as the canonical "full custom run" entry. Pills are the power-user shortcut layer.

**Cost:** new persistence surface (preset library), new settings UI to manage presets. Worth it once the user has 3+ runs they repeat weekly.

### FC-3 · Cost meter expands to a stacked spend bar

Hover or click the top-bar cost meter to expand it into a stacked horizontal bar that breaks down the session spend by source: each running terminal and run gets a colored segment proportional to its share of the total. Hover a segment → tooltip with the session ID + exact USD.

Turns the cost meter from a single number into a "where is my money going right now" view — without taking permanent screen real estate.

**Cost:** requires backend to attribute USD-per-segment in real time, not just total.

## Decisions Log

| Date | Decision | Rationale |
|---|---|---|
| 2026-05-13 | Initial system created via /design-consultation | Slack-style IA + harbormaster (cockpit) frame + signal palette + Plex Condensed/Mono pairing + harbor cyan chrome accent + Manifest Strip + loud cost meter |
| 2026-05-13 | Reframed cockpit → harbormaster | Subagent critique: cockpit = single vehicle / tight loop. Product = fleet dispatch / ambient awareness. Different IA pressure. |
| 2026-05-13 | Rejected mono-only UI | Subagent critique: mono-everywhere is LARP, screenshot-optimized not 9th-hour-of-use optimized. Bloomberg uses condensed sans + mono tickers, not mono-everywhere. |
| 2026-05-13 | Amber reserved for `running` only | Subagent critique: status colors must not double as chrome accents — desensitization within an hour. Harbor cyan handles chrome instead. |
| 2026-05-13 | Plex Condensed over Inter / Geist | Anti-convergence: 80% of 2026 dev tools default to Inter family. Plex Condensed has typographic voice + same density. |
| 2026-05-13 | Plex Mono over JetBrains Mono / Commit Mono | Native pairing with Plex Condensed (same designer, same x-height). Underused. Free on Google Fonts. |
| 2026-05-13 | Reviewed Agent Twin prototype, rejected overall direction | Light cream + chat REPL + bubble radius conflicts with harbormaster brief. Single-agent chat metaphor can't show 5 terminals + 3 runs simultaneously. |
| 2026-05-13 | Queued 3 Agent Twin moves as v0.2 future considerations | Repo-card sidebar header (FC-1), slash-command pills for quick-launch (FC-2), stacked spend bar on cost-meter expand (FC-3). See Future Considerations section above. |

## Preview

The full design system rendered as a working HTML page lives at [`docs/design-preview.html`](docs/design-preview.html). Open it in any browser — light/dark toggle in the top right.

A point-in-time consultation audit (decisions, departures, subagent critiques applied) is kept outside the repo at `~/.gstack/projects/liyoclaw1242-agent-sweet-home/designs/design-system-20260513/approved.json`.

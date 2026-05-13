# Agent Sweet Home — Claude operating notes

This is a Tauri 2 + React 19 + TypeScript desktop app. See `README.md` for the full product description and `WORKFLOW.md` for the workflow engine spec.

## Design System

**Always read `DESIGN.md` before making any visual or UI decision.** All font choices, colors, spacing, aesthetic direction, anti-slop rules, and signature layout moves (Manifest Strip, loud cost meter, status-colored sidebar rails, log weather sparkline) are defined there.

Do not deviate from `DESIGN.md` without explicit user approval. In QA / review mode, flag any code that doesn't match `DESIGN.md`.

**Hard guard rails** (full list in `DESIGN.md` → Anti-Slop Hard Rules):

- Two faces only: `IBM Plex Sans Condensed` (chrome) + `IBM Plex Mono` (numerics / IDs / logs / xterm). Never `Inter`, `Roboto`, `Helvetica`, `Geist`, `Space Grotesk`, or `system-ui`.
- Status colors (`#F5A524` running, `#3FB950` healthy, `#F85149` failed, `#7C8AA0` frozen, `#FF6B35` cost-burn) are **reserved for semantics**. Never use them for buttons, links, focus, or selection.
- Chrome accent is `#5BC0EB` (harbor cyan). It carries focus, selection, brand, links — and nothing else.
- `border-radius: 2px` global maximum. `0px` on terminal frame and tables. No bubble radius.
- No purple. No gradients on chrome. No drop-shadow elevation (use border steps).
- Mono is **surgical**, not system. Sidebar labels and body text are sans.

## Skill routing

When the user's request matches an available skill, invoke it via the Skill tool. The skill has multi-step workflows, checklists, and quality gates that produce better results than an ad-hoc answer. When in doubt, invoke the skill.

Key routing rules:

- Product ideas, "is this worth building", brainstorming → invoke /office-hours
- Strategy, scope, "think bigger", "what should we build" → invoke /plan-ceo-review
- Architecture, "does this design make sense" → invoke /plan-eng-review
- Design system, brand, "how should this look" → invoke /design-consultation
- Design review of a plan → invoke /plan-design-review
- Developer experience of a plan → invoke /plan-devex-review
- "Review everything", full review pipeline → invoke /autoplan
- Bugs, errors, "why is this broken", "wtf", "this doesn't work" → invoke /investigate
- Test the app, find bugs, "does this work" → invoke /qa (or /qa-only for report only)
- Code review, check the diff, "look at my changes" → invoke /review
- Visual polish, design audit, "this looks off" → invoke /design-review
- Developer experience audit, try onboarding → invoke /devex-review
- Ship, deploy, create a PR, "send it" → invoke /ship
- Merge + deploy + verify → invoke /land-and-deploy
- Update docs after shipping → invoke /document-release
- Weekly retro, "how'd we do" → invoke /retro
- Save progress, "save my work" → invoke /context-save
- Resume, restore, "where was I" → invoke /context-restore
- Security audit, OWASP → invoke /cso
- Code quality dashboard → invoke /health

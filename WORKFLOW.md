<!-- audience: AI agents authoring/modifying workflow YAML or implementing the
     Phase-2 runtime against this schema. NOT for human readers — drop the
     marketing prose and read the type table directly. Truth source is
     src-tauri/src/workflow/spec.rs; if this doc disagrees with that file,
     spec.rs wins. -->

# Workflow Engine — Reference

Declarative YAML+Jinja2 engine for issue→directive dispatch and per-role
result handling. Models the same surface as the agent-team v2 supervisor
(`dispatcher.ts` + `orchestrator.ts`) but as a config-driven runtime.

## STATUS

Phase 1.5. Schema accepts the production v2 YAML; runtime is stubs.

| Layer                       | State    | File                              |
| --------------------------- | -------- | --------------------------------- |
| YAML deserialization        | DONE     | `src-tauri/src/workflow/spec.rs`  |
| Pure dispatch               | DONE     | `src-tauri/src/workflow/dispatch.rs` |
| Expression engine (minijinja) | DONE   | `src-tauri/src/workflow/expr.rs`  |
| Template renderer (`{var}`) | DONE     | `src-tauri/src/workflow/command.rs` (`render_template`) |
| `run_capture` / `run_capture_json` | STUB | `src-tauri/src/workflow/command.rs` |
| `apply_on_result` / `execute_steps` | STUB | `src-tauri/src/workflow/result.rs` |
| `apply_degrade` / `apply_unblock_pass` | STUB | `src-tauri/src/workflow/result.rs` |
| Entry: poll                 | STUB     | `src-tauri/src/workflow/entry/poll.rs` |
| Entry: webhook              | STUB     | `src-tauri/src/workflow/entry/webhook.rs` |
| Entry: manual               | STUB     | `src-tauri/src/workflow/entry/manual.rs` |
| `Workflow::run(EntryMode)`  | NOT WIRED | `src-tauri/src/workflow/mod.rs`  |
| Tauri commands              | NOT WIRED | n/a (no `workflow_*` invokes registered) |

`STUB` ≡ function signature exists, body is `todo!()`. `PARTIAL` ≡ some
sub-functions implemented, others `todo!()`. Run `rg 'todo!\(' src-tauri/src/workflow/`
to enumerate.

## TOP-LEVEL YAML KEYS

| Key                       | Req | Rust type at `spec.rs`                  | Purpose                                       |
| ------------------------- | --- | --------------------------------------- | --------------------------------------------- |
| `version`                 | yes | `u32`                                   | Schema version (currently 1).                 |
| `entry`                   | yes | `EntryConfig`                           | When/how to fetch issues.                     |
| `roles`                   | yes | `HashMap<String, RoleConfig>`           | Per-role spawn config keyed by role name.     |
| `dispatch`                | yes | `DispatchConfig`                        | `(IssueState, [DispatchRule]) → Directive`.   |
| `pre_spawn`               | no  | `Vec<PreSpawnHook>`                     | Hooks fired AFTER `spawn_fresh` resolves, BEFORE the subprocess starts. |
| `on_result`               | no  | `HashMap<String, Vec<KindHandler>>`     | Branch on `(role, out.kind)`. Deserialized from a flat YAML list and grouped by role. |
| `on_no_structured_output` | no  | `Option<StepBlock>`                     | Degrade fallback when a spawn finishes but emits nothing parseable. |
| `unblock_pass`            | no  | `UnblockConfig`                         | Promote `status:blocked` → `status:ready` when dep issues close. |
| `shared_templates`        | no  | `HashMap<String, String>`               | Reusable Jinja fragments referenced via `body_template: __shared.<name>`. |

Unknown keys are silently dropped (default serde behavior). Use
`rg --type=rust "WorkflowFile|struct Workflow"` to confirm.

## ENTRY (`entry:`)

Three modes share the same dispatch + on_result tables. `entry.modes`
declares which loops the runtime starts; `entry.<mode>` blocks supply
per-mode config.

```yaml
entry:
  modes: [poll, manual]   # may include any of: poll | webhook | manual

  poll:
    interval_sec: 30
    max_in_flight: 4              # concurrent dispatches per tick
    repo_source:
      command: "agent-sweet-home registry list --json"
      # → JSON: [{"repo":"o/r","path":"/abs/path"}, ...]
    issue_source:
      command: |
        gh issue list --repo {repo} --state open
          --json number,title,body,labels,state --limit 100

  webhook:
    enabled: false                # off by default; runtime no-ops when false
    listen: "0.0.0.0:8787"
    secret_env: GITHUB_WEBHOOK_SECRET
    events: [issues, issue_comment, label]
    issue_source:
      command: |
        gh issue view {issue_number} --repo {repo}
          --json number,title,body,labels,state

  manual:
    issue_source:
      command: |
        gh issue view {issue_number} --repo {repo}
          --json number,title,body,labels,state
```

Source-command template variables: `{repo}`, `{issue_number}`. Substitution
is a literal string replace (`command::render_template`); Jinja2 is NOT
applied to entry commands. Whitespace inside braces is tolerated
(`{ repo }` ≡ `{repo}`).

The runtime parses returned JSON and derives `issue.markers` from
`<!-- key: value -->` HTML comments in `issue.body` — markers are NOT
returned by GitHub; they live in body text.

## ROLES (`roles:`)

```yaml
roles:
  implementer:
    system_prompt_file: v2/agents/implementer.md   # path resolved relative to YAML
    add_dirs:
      - v2/skills/implementer
      - "{repo_path}/project-rlm"                  # `{repo_path}` substituted at spawn time
    allowed_tools: ["Read", "Edit", "Write", "Bash(git *)"]
    disallowed_tools: ["Edit", "Write"]            # applied AFTER allowed_tools
    json_schema_file: v2/schemas/implementer.json
    budget_usd: 2.5
    model: claude-sonnet-4-6                       # optional; runtime default if absent
    needs_worktree: true
    mode_overrides:
      review:
        allowed_tools: ["Read", "Glob", "Grep"]
        disallowed_tools: ["Edit", "Write"]
        # `model` and `budget_usd` also overridable
```

`mode_overrides.<mode>` swaps fields when a `Directive::SpawnFresh.mode`
matches. Only listed fields override; others inherit.

## DISPATCH (`dispatch.rules:`)

Top-down evaluation; **first match wins**. No-match → implicit
`Directive::NoAction { reason: "no rule matched" }`.

```yaml
dispatch:
  rules:
    - when: { issue_state: closed }
      then: { directive: no_action, reason: "closed" }

    - when:
        all:
          - has_label: status:ready
          - has_label: agent:arch-shape
          - expr: "(markers['mode-c-rounds'] | default('0') | int) >= 3"
      then: { directive: human_review, reason: "Mode C rounds reached cap" }

    - when:
        all: [{ has_label: status:ready }, { has_label: agent:implementer }]
      then: { directive: spawn_fresh, role: implementer }
```

### Predicate atoms (`AtomPredicate` @ `spec.rs:121`)

| YAML key             | Value type | Meaning                                           |
| -------------------- | ---------- | ------------------------------------------------- |
| `has_label`          | string     | Exact label match.                                |
| `matches_label`      | string     | Prefix match (e.g. `"status:"` matches `status:ready`). |
| `not_has_label`      | string     | Sugar for `not: { has_label: ... }`.              |
| `issue_state`        | `"open"` \| `"closed"` | State of the issue.                   |
| `has_marker`         | string     | True if body contains `<!-- key: ... -->`.        |
| `expr`               | string (Jinja) | Arbitrary boolean expression. See EXPRESSIONS. |
| `repo_path_exists`   | bool       | Pre-spawn only. False in dispatch context.        |
| `path_exists`        | string (path) | Pre-spawn only. False in dispatch context.     |
| `role`               | string     | Pre-spawn only. Matches the resolved spawn role. False in dispatch context. |

### Combinators

```yaml
all: [<pred>, <pred>, ...]    # AND, short-circuits on first false
any: [<pred>, <pred>, ...]    # OR, short-circuits on first true
not: <pred>                   # negation; expects a single nested predicate
```

Combinators nest arbitrarily.

### Directives (`Directive` @ `spec.rs:171`)

| `directive:` value | Required fields    | Optional fields | Meaning                                   |
| ------------------ | ------------------ | --------------- | ----------------------------------------- |
| `no_action`        | —                  | `reason`        | Skip this issue, no side effects.         |
| `wait`             | —                  | `reason`        | In-flight; no spawn, no labels.           |
| `human_review`     | —                  | `reason`        | Flag for human; runtime adds `human-review` label idempotently before returning, so the dispatch loop doesn't re-fire. |
| `spawn_fresh`      | `role`             | `mode`, `reason` | Start new claude subprocess with the role's config. |

## PRE-SPAWN (`pre_spawn:`)

Sequential hook list. Each hook has YAML aliases: `if:` for the predicate,
`do:` for the steps. Runs after `spawn_fresh` resolves but before the
subprocess starts. Hooks may abort or reroute the spawn.

```yaml
pre_spawn:
  - if: { not: { repo_path_exists: true } }
    do:
      - comment: |
          repo not cloned at `{{ repo_path }}` — run `git clone https://github.com/{{ repo }} {{ repo_path }}`
      - transition_status: { from: ready, to: blocked }
      - add_labels: ["human-review"]
      - abort_spawn: true

  - if:
      all:
        - role: arch-shape
        - not: { path_exists: "{repo_path}/project-rlm" }
    do:
      - reroute: { role: rlm-modeler, mode: bootstrap }
```

`abort_spawn: true` halts the pipeline. `reroute: { role, mode? }` swaps
the spawn target; later hooks see the rerouted role.

## ON-RESULT (`on_result:`)

YAML is a flat list of `{ role, when:{kind}, steps }`. In memory,
serde groups by role into `HashMap<role, Vec<KindHandler>>` for O(1)
`(role, kind)` lookup. **Multiple entries per role are allowed and
expected** — one per `out.kind` variant.

```yaml
on_result:
  - role: arch-shape
    when: { kind: decomposition }
    steps:
      - bind:
          rlm_changes: "out.rlm_changes_needed | default([])"
      - if: "rlm_changes | length > 0"
        steps:
          - create_issue:
              title: "[rlm-modeler] Apply {{ rlm_changes | length }} RLM change(s)"
              body: "{{ rlm_changes | tojson(indent=2) }}"
              labels: ["agent:rlm-modeler", "status:ready"]
            bind: rlm_child

  - role: arch-shape
    when: { kind: rejected }
    steps:
      - add_labels: ["human-review"]
      - transition_status: { from: in-progress, to: blocked }
```

`out.kind` matching is **exact**. No wildcards, no fall-through.

## ACTIONS (`ActionInput` @ `spec.rs:298`)

Externally tagged: each step is a one-key map (`create_issue:`, `comment:`,
`add_labels:`, …). A sibling `bind: <var>` key on any action captures its
result into the bindings context (see STEP-LEVEL SIBLING BIND).

| YAML key             | Value shape                                                   | Notes                                                             |
| -------------------- | ------------------------------------------------------------- | ----------------------------------------------------------------- |
| `create_issue`       | `{ title, body?, body_template?, labels?, deps? }`            | `labels`: list of strings OR `{ if: <expr>, label: <str> }` entries. `deps`: list of expressions OR a single Jinja expression yielding an array. |
| `comment`            | scalar string OR `{ body }` OR `{ template }`                 | All forms render via Jinja2 at runtime (Phase 2).                |
| `add_labels`         | list of strings — `["a", "b"]`                                | NOT `{ labels: [...] }`. Direct list.                            |
| `remove_labels`      | list of strings                                               | Same shape as `add_labels`.                                       |
| `transition_status`  | `{ from?: str, to: str }`                                     | `from` is optional; runtime asserts current state if provided.   |
| `set_body_marker`    | arbitrary `{ key1: val1, key2: val2 }` map                    | Each pair becomes `<!-- key: value -->` in issue body.           |
| `push_branch_and_pr` | `{ branch?, base, title, body_template?, closes_issue?, post_merge_note? }` | `closes_issue` accepts `bool` OR Jinja string expression. |
| `run_command`        | `{ argv: [str], cwd?, stdin?, bind_stdout?, bind_exit? }`     | Spawns a process. Bind names capture stdout/exit-code.           |
| `abort_spawn`        | `true`                                                        | Pre-spawn only.                                                   |
| `reroute`            | `{ role, mode? }`                                             | Pre-spawn only.                                                   |

### Examples

```yaml
# create_issue with conditional label and string-expr deps
- create_issue:
    title: "[{{ task.subdomain }}] {{ task.spec.split('\n')[0] }}"
    labels:
      - "agent:implementer"
      - "{{ 'status:blocked' if blocked else 'status:ready' }}"
      - if: "task.severity is defined"
        label: "severity:{{ task.severity }}"
    deps: "{{ resolved_dep_numbers }}"     # single expr → runtime array
  bind: child_issue

# comment scalar form (most common)
- comment: |
    decomposition complete — {{ children | length }} child task(s) opened

# multi-key set_body_marker
- set_body_marker:
    arch-advisors-spawned: "{{ out.advisors | join(',') }}"
    arch-consultation-brief: "{{ out.consultation_brief }}"

# push_branch_and_pr with expr closes_issue and post-merge note
- push_branch_and_pr:
    branch: "{{ out.branch }}"
    base: main
    title: "[bootstrap] {{ issue.title }}"
    body_template: __shared.delivery_pr_body
    closes_issue: "{{ not is_bootstrap }}"      # Jinja string → BoolOrExpr::Expr
    post_merge_note: |
      Bootstrap PR — does NOT close issue #{{ issue.number }}.
  bind: pr
```

## CONTROL FLOW (`ControlFlow` @ `spec.rs:262`)

### `if` / `elif` / `else`

```yaml
- if: "out.verdict == 'approve'"
  steps:
    - transition_status: { to: done }
  elif:                                                      # nested list
    - elif: "out.verdict == 'request_changes'"
      steps:
        - add_labels: ["human-review"]
  else:
    - transition_status: { to: ready }
```

`elif` is a **list of `{elif, steps}` objects**, NOT a flat key-pair after
the parent. Flat `if/steps/elif/steps/else` is invalid YAML (duplicate
`steps:` key — `serde_yaml` rejects). See GOTCHAS.

### `for_each`

```yaml
- for_each: "out.child_tasks"
  as: task
  steps:
    - create_issue:
        title: "{{ task.subdomain }}"
        ...
      bind: child_issue                  # captured PER ITERATION
```

Iteration accumulator runtime contract (Phase 2):

| Variable             | Visible inside loop | Visible outside |
| -------------------- | ------------------- | --------------- |
| `task` (loop var)    | yes                 | no              |
| `_iter_results`      | yes (auto-tracked)  | no              |
| `_last_iter_results` | no                  | yes (snapshot of inner-most loop) |

### `bind` (control-flow form, multi-key)

```yaml
- bind:
    findings: "out.findings | default([])"
    ac_coverage: "out.ac_coverage | default([])"
```

Each value is a Jinja expression. `bind:` is the only top-level key. The
result mutates `bindings.<name>` for subsequent steps in the same scope.

### Step-level sibling `bind` (action form)

```yaml
- create_issue: { title: "...", labels: [...] }
  bind: rlm_child
```

Sibling `bind:` is a String, not a map. It captures the action's structured
result. Distinguished from control-flow `bind:` by value shape:

| Shape                 | Parses as          |
| --------------------- | ------------------ |
| `bind: <map>`         | `ControlFlow::Bind` |
| `<action>:` + `bind: <str>` (siblings) | `ActionStep { action, bind: Some(<str>) }` |
| `bind: <str>` alone   | parse error        |

Implementation: `ActionStep` flattens `ActionInput` and adds an optional
`bind: Option<String>` field. See `spec.rs:248`.

## EXPRESSIONS

Expression engine: minijinja 2.x with custom filters. See `expr.rs`.

### Built-in context variables (Phase 2 contract)

| Variable          | Type         | Source                                          |
| ----------------- | ------------ | ----------------------------------------------- |
| `issue`           | IssueSnapshot | Snapshot of the dispatched issue.              |
| `issue.number`    | u64          | —                                               |
| `issue.title`     | string       | —                                               |
| `issue.body`      | string       | —                                               |
| `issue.state`     | string       | `"open"` \| `"closed"`                          |
| `issue.labels`    | list of str  | —                                               |
| `issue.markers`   | dict<str,str> | Parsed from `<!-- key: value -->` body comments. |
| `out`             | JSON         | Spawn structured output (only set in on_result). |
| `bindings.<name>` | any          | Set by `bind:` steps.                           |
| `repo`            | string       | `"owner/name"`. **Phase 2 must add to ExprContext.** |
| `repo_path`       | string       | Local clone abs path. **Phase 2 must add.**     |
| `spawn`           | dict         | Set in `on_no_structured_output` only. Fields: `role`, `end_reason`, `cost_usd`, `duration_ms`, `last_assistant_text`, `stderr`. **Phase 2 must add.** |

### Built-in helper filters / tests (registered today)

`has_label(name)`, `matches_label(prefix)`, `not_has_label(name)`,
`has_marker(key)` — exposed as both filter and global function.

### Custom filters registered in `expr.rs:ExprEngine::new()`

| Filter / Function           | Purpose                                                    |
| --------------------------- | ---------------------------------------------------------- |
| `formatdep`                 | Render an issue number as `#N` (used in dep summary lines). |
| `lookup_iter_result_number` | Inside `for_each`, map an int index to the prior iteration's created-issue number. |
| `asint`                     | Coerce string to int. Used to resolve dep array indices.   |

**Core (always available)**: `tojson`, `length`, `default`, `attribute`,
`split`, `int`, `join`, `trim`, `format`, `lower`, `upper`, `escape` /
`safe` come from minijinja core.

**Contrib (registered in `ExprEngine::new` via `minijinja_contrib::add_to_environment`)**:
`truncate`, `wordwrap`, `pluralize`, `pyfromstr`, `slug`, plus date/time helpers.

⚠️ minijinja-contrib's `truncate` is **kwargs-only**. Use
`{{ s | truncate(length=500, end='…') }}`, NOT Jinja2's positional form
`truncate(500, '…')` — the latter raises `TooManyArguments`.

`groupby` / `map` / `selectattr` / `reject` are *not* in either crate today;
authors who need them have to inline an `if` test or use `expr.eval_value`
on a richer expression.

## TEMPLATE NAMESPACES

`body_template:` accepts two forms:

| Form                                | Resolution                                         |
| ----------------------------------- | -------------------------------------------------- |
| `templates/foo.md`                  | File path, relative to the workflow YAML directory. |
| `__shared.<name>`                   | Look up `Workflow.shared_templates[<name>]`.        |

`__shared.` prefix is a runtime convention. `expr.rs::ExprEngine::render_body_template`
checks the prefix and substitutes from `wf.shared_templates` before
rendering. Falls back to inline rendering when the prefix is absent or
the named entry is missing. Action handlers that accept `body_template:`
(`create_issue`, `comment` `template`, `push_branch_and_pr`) call
`render_body_template` so the prefix works uniformly.

## UNBLOCK PASS (`unblock_pass:`)

```yaml
unblock_pass:
  enabled: true
  on_unblock:
    - comment: |
        🔓 Deps satisfied — promoting to status:ready
    - remove_labels: ["status:blocked"]
    - add_labels: ["status:ready"]
```

Runs once per `pollOnce()` BEFORE dispatch. For every `status:blocked`
issue, parse `<!-- deps: #X #Y -->` from body, check each dep is closed,
and run `on_unblock` steps when all are.

## DEGRADE FALLBACK (`on_no_structured_output:`)

Fired when a spawn finishes but emits no parseable JSON. Reuses `Step`
syntax. Has a special context — `spawn.*` is populated:

```yaml
on_no_structured_output:
  steps:
    - comment: |
        ❌ Spawn {{ spawn.role }} produced no structured output

        End reason: {{ spawn.end_reason }}
        Cost: ${{ '%.4f' | format(spawn.cost_usd) }}
        Duration: {{ '%.1f' | format(spawn.duration_ms / 1000) }}s

        Last assistant text:
        ```
        {{ spawn.last_assistant_text | default('(none)') | truncate(length=1500, end='') }}
        ```
    - add_labels: ["human-review"]
    - transition_status: { from: in-progress, to: blocked }
```

## GOTCHAS

| Symptom                                                                                                 | Cause                                                                                                 | Fix                                                                                |
| ------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| `Error("duplicate entry with key \"steps\"", line: N)`                                                  | Flat `if/steps/elif/steps/else` shape (duplicate `steps:` keys). YAML 1.2 invalid; serde_yaml rejects. | Use nested form: `elif: [{ elif: <expr>, steps: [...] }, ...]`.                    |
| `pre_spawn[N].do: data did not match any variant of untagged enum Step`                                 | Action key not recognized OR sibling-bind on a control-flow step.                                     | Check action key snake_case; control-flow steps don't take sibling `bind:`.        |
| `add_labels: ["x"]` parses fine; `add_labels: { labels: ["x"] }` fails                                  | `AddLabels` is a tuple variant, not a struct. Same for `remove_labels`.                               | Always use direct list form.                                                       |
| `closes_issue: true` parses; `closes_issue: "{{ expr }}"` parses; `closes_issue: { ... }` fails         | `BoolOrExpr` is `bool | string`, not a map.                                                          | Use bool literal or Jinja string only.                                             |
| `comment: { body: ..., template: ... }` parses but rendering ambiguous                                  | Schema accepts both fields; runtime semantics not defined.                                            | Pick one. Phase-2 runtime should pick `template` if present, else `body`.          |
| `on_result` entries appear to silently merge across roles                                               | Multiple `{ role, when:{kind}, steps }` entries with same `(role, kind)` — last wins after grouping.  | Each `(role, kind)` pair must be unique. Test-cover this if you author handlers.   |
| `repo_path_exists` / `path_exists` / `role` predicates always evaluate `false` in dispatch              | They're pre-spawn-only atoms. `dispatch.rs` returns `false` for them by design (`dispatch.rs:81`).    | Use them in `pre_spawn:` only, never in `dispatch.rules:`.                         |
| `shared_templates:` block silently dropped                                                              | Pre-Phase-1.5 schema didn't have the field; you might be on an old build.                              | Confirm `cargo test workflow::spec::tests::shared_templates_section_is_captured` passes. |
| `entry:` missing → schema parse error                                                                   | `entry` is required since Phase 1.5.                                                                   | Add at least `entry: { modes: [manual], manual: { issue_source: { command: "..." } } }`. |
| `run_command` runs in tauri pwd, not the worktree, so `git add SMOKE.md` errors `pathspec did not match` | `run_command` defaults `cwd:` to `rt.repo_path` (worktree path for `needs_worktree` roles, canonical clone otherwise). If you wrote it before the default was added, an unset `cwd:` ran in tauri's pwd. | Upgrade to engine ≥ smoke-prep-3, or set `cwd: "{{ worktree_path }}"` explicitly. |
| `push_branch_and_pr: branch unset, out.branch missing, bindings.branch missing`                          | Action expected a branch but YAML didn't say `branch:`, agent JSON didn't include `branch`, AND `bindings.branch` was empty (i.e. role had `needs_worktree: false`).                                       | Either set `branch: "{{ branch }}"` (worktree role) or include `branch` in the agent's JSON output. |
| `truncate(500, end='…')` errors `TooManyArguments`                                                       | minijinja-contrib's `truncate` is **kwargs-only**, unlike Jinja2's positional form.                                                                                                                       | Use `truncate(length=500, end='…')`.                                              |
| Issue keeps re-spawning every poll tick after a dispatch error                                           | Pre-smoke-prep-3, on_result errors didn't quarantine. Now the poll loop adds `human-review` and removes `status:ready` on dispatch error so the next tick's rules don't match.                            | Upgrade engine. If you see the loop recur after that, dispatch error itself is leaking past the catch — file a bug.       |

## TEST FIXTURES

Canonical YAML examples live in `src-tauri/src/workflow/spec.rs` `#[cfg(test)] mod tests`:

| Test name                                              | Demonstrates                                                  |
| ------------------------------------------------------ | ------------------------------------------------------------- |
| `parses_minimal_workflow`                              | Smallest valid YAML.                                          |
| `entry_parses_all_three_modes_and_source_commands`     | Full `entry:` shape with poll/webhook/manual.                |
| `on_result_groups_by_role`                             | List → HashMap grouping.                                      |
| `parses_compound_predicates`                           | `all` / `any` / `not` nesting.                                |
| `parses_steps_mixing_control_and_actions`              | All step variants in one handler.                             |
| `pre_spawn_atoms_repo_path_exists_and_path_exists_and_role` | Pre-spawn-only atoms.                                    |
| `abort_spawn_and_reroute_actions`                      | Pre-spawn-only actions.                                       |
| `comment_accepts_scalar_and_detailed_forms`            | Scalar block / `body` / `template` forms.                     |
| `create_issue_supports_conditional_labels_and_string_deps` | LabelEntry + DepsValue variants.                          |
| `set_body_marker_takes_arbitrary_multi_key_map`        | Multi-key marker shape.                                       |
| `push_branch_and_pr_accepts_branch_and_expr_closes_issue` | All optional fields exercised.                            |
| `role_config_parses_mode_overrides`                    | `mode_overrides.<mode>` shape.                                |
| `shared_templates_section_is_captured`                 | `__shared.<name>` storage.                                    |
| `unblock_pass_captures_on_unblock_steps`               | `on_unblock:` step list.                                      |
| `nested_elif_branches_round_trip`                      | Correct elif shape (NOT the flat one).                        |
| `parses_real_agent_team_workflow` (`#[ignore]`d)       | Integration: parse `~/Projects/agent-team/agent-team-v2.workflow.yaml`. Run with `cargo test -- --ignored`. |

When adding a new schema feature, also add a fixture test that mirrors the
prod YAML shape verbatim (with quadruple-brace escaping for `{{ var }}`
inside `format!()` macros — see existing tests).

## PHASE 2 IMPLEMENTATION CONTRACT

Each `todo!()` must satisfy the contract in its docstring. Summary:

### `command::run_capture(rendered: &str) -> Result<Vec<u8>>`

- Spawn `sh -c <rendered>` with timeout from runtime config.
- Capture stdout. Return non-zero exit as `CommandError::NonZeroExit`.

### `command::run_capture_json<T: DeserializeOwned>(rendered) -> Result<T>`

- Wraps `run_capture` + `serde_json::from_slice`.
- Surface JSON parse errors as `CommandError::Json`.

### `entry::poll::run_poll_loop(cfg, shutdown)`

- `tokio::time::interval(cfg.interval_sec)` ticks.
- Per tick: `PollSource::fetch_repos()` → `JoinSet`-bounded fan-out per
  repo → `fetch_issues(repo)` → for each issue, dispatch via
  `dispatch::dispatch()` and execute via `result::apply_on_result()` (or
  `apply_degrade()` on no-output).
- `tokio::select!` on `shutdown` for graceful exit.
- Bound concurrency to `cfg.max_in_flight` via `tokio::sync::Semaphore`.

### `entry::webhook::run_webhook_listener(cfg, shutdown)`

- Early-return `Ok(())` when `!cfg.enabled`.
- Bind axum to `cfg.listen`. Single POST handler:
  - Verify HMAC against `std::env::var(cfg.secret_env)`. Reject 401 on mismatch.
  - Filter `X-GitHub-Event` against `cfg.events`. 204 for unsubscribed events.
  - Resolve issue via `WebhookSource::fetch_one(repo, n)` → dispatch → execute.

### `entry::manual::run_one(cfg, repo, n)`

- Synchronous-feel entry for CLI / Tauri command. No loop, no
  concurrency. Resolve via `ManualSource::fetch_one` → dispatch → execute.
- Surface errors via the return value; do not panic.

### `result::execute_steps(steps, ctx, engine)`

Single source of truth shared by `apply_on_result`, `apply_pre_spawn`,
`apply_unblock`, `apply_degrade`. Pattern-match each `Step`:

| `Step` variant                                  | Action                                                               |
| ----------------------------------------------- | -------------------------------------------------------------------- |
| `Control::If { condition, steps, elif, else_steps }` | Eval `condition` via `engine.eval_bool`. Recurse on the matching branch. |
| `Control::ForEach { iter_expr, var, steps }`    | Eval `iter_expr` to array. For each element, set `var`, push results into `_iter_results`, recurse. After loop, set `_last_iter_results` to the array. |
| `Control::Bind { bind }`                        | Eval each value via `engine.eval_value`. Insert into `ctx.bindings`. |
| `Action(ActionStep { action, bind })`           | Dispatch `action` to its executor (Phase 2). If `bind.is_some()`, store the action's structured result into `ctx.bindings[<name>]`. |

### `result::apply_on_result(wf, role, kind, out, ctx, engine)`

- Look up handler via `wf.on_result_for(role)?.iter().find(|h| h.kind == kind)`.
- Inject `out` into `ctx.out`.
- Call `execute_steps` on `handler.steps`.
- If no handler matches, return `ResultError::NoHandlerMatched`.

### Action executors (Phase 2 — not yet sketched)

Each action variant maps to one side-effect routine. The runtime owns:

- a GitHub writer (REST or `gh` CLI, mirror entry mode style),
- a worktree manager for `needs_worktree: true` roles,
- a spawn bridge that hands the rendered prompt + tool config to `claude -p`.

`AbortSpawn` and `Reroute` mutate the pre-spawn pipeline state (a struct
flowing alongside `ExprContext` in pre-spawn evaluation only). They have
no on_result semantics — author error if they appear there.

## CONVENTIONS

- All YAML keys are `snake_case`. Rust enum variants match via
  `#[serde(rename_all = "snake_case")]`.
- Markers in body comments: lowercase keys, hyphens allowed, e.g.
  `<!-- arch-state: awaiting-consultation -->`.
- Status labels: `status:<state>` prefix. Used by `matches_label: "status:"`.
- Role/agent labels: `agent:<role>` prefix.
- Substate labels: `<role>-state:<state>` (e.g. `arch-state:awaiting-consultation`).

## GLOSSARY

| Term                  | Meaning                                                                  |
| --------------------- | ------------------------------------------------------------------------ |
| `IssueSnapshot`       | Snapshot of one issue passed to expressions. See `expr.rs:23`.           |
| `Directive`           | Output of `dispatch::dispatch()`. See `spec.rs:171`.                     |
| `Step`                | Either a control-flow construct or an action. See `spec.rs:233`.         |
| `ActionStep`          | An `ActionInput` + optional sibling `bind`. See `spec.rs:248`.           |
| `KindHandler`         | One on_result branch matched by `out.kind`. See `spec.rs:217`.           |
| `BoolOrExpr`          | Boolean OR Jinja expression — used for `closes_issue`. See `spec.rs:401`. |
| `LabelEntry`          | Plain string OR `{ if, label }` conditional. See `spec.rs:381`.          |
| `DepsValue`           | List of expressions OR single expression yielding array. See `spec.rs:393`. |
| `CommentBody`         | Scalar string OR `{ body, template }` map. See `spec.rs:367`.            |
| Markers               | `<!-- key: value -->` HTML comments embedded in issue body for runtime-mutable state. |
| Pre-spawn-only        | Atoms / actions that have meaning only in `pre_spawn:` hooks. Evaluated as `false` (atoms) or are author errors (actions) elsewhere. |

## CHANGELOG (schema-relevant)

| Version    | Change                                                                                                       |
| ---------- | ------------------------------------------------------------------------------------------------------------ |
| Phase 1    | Initial schema: `roles`, `dispatch.rules`, `pre_spawn`, `on_result` (as `Vec<ResultHandler>`), `unblock_pass`, action variants. |
| Phase 1.5  | + `entry:` block. + `repo_path_exists` / `path_exists` / `role` atoms. + `abort_spawn` / `reroute` actions. + `mode_overrides`. + `shared_templates`. + `unblock_pass.on_unblock`. `on_result` regrouped by role. `add_labels` / `remove_labels` → tuple variants. `comment` accepts scalar form. `set_body_marker` → multi-key map. `push_branch_and_pr` adds `branch`, `closes_issue` → `BoolOrExpr`, `post_merge_note` (renamed from `post_merge_note_template`). `create_issue.deps` → `DepsValue`. `create_issue.labels` → `Vec<LabelEntry>`. Sibling-key `bind:` for actions. `WorkflowFile` renamed to `Workflow`. |

When extending the schema, append a row here AND a fixture test in
`spec.rs` AND update the relevant section above.

//! YAML → Rust types for a workflow file. Pure deserialization layer; no
//! runtime semantics live here. Phase-1 framing of design § 2 + Phase-1.5
//! widening for the agent-team v2 production YAML.
//!
//! Decisions worth flagging for later phases:
//! - `Predicate`, `Step`, `ControlFlow` and `CommentBody` are
//!   `#[serde(untagged)]` so the YAML reads naturally — a step is just
//!   `- if: "expr"` / `- create_issue: {…}`, and `comment:` accepts both a
//!   scalar block and a `{ template, body }` map. Variants get inferred from
//!   which key (or shape) is present.
//! - `ActionInput` stays externally tagged with `rename_all = "snake_case"`
//!   — each step is a one-key map (`create_issue`, `comment`, …). It is
//!   wrapped in `ActionStep` so a sibling `bind:` key (`- create_issue: {…}\n
//!   bind: pr`) can capture the action's result without nesting it inside
//!   the action body.
//! - `on_result` deserializes from a YAML list of `{ role, when:{kind}, steps }`
//!   into `HashMap<role, Vec<KindHandler>>` for O(1) (role,kind) dispatch at
//!   runtime. The flat list form is preserved at the YAML layer for clarity;
//!   the in-memory shape is grouped.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct Workflow {
    pub version: u32,
    pub entry: EntryConfig,
    pub roles: HashMap<String, RoleConfig>,
    pub dispatch: DispatchConfig,
    #[serde(default)]
    pub pre_spawn: Vec<PreSpawnHook>,
    #[serde(default, deserialize_with = "deserialize_on_result")]
    pub on_result: HashMap<String, Vec<KindHandler>>,
    #[serde(default)]
    pub on_no_structured_output: Option<StepBlock>,
    #[serde(default)]
    pub unblock_pass: UnblockConfig,
    /// Reusable Jinja fragments referenced via `body_template:
    /// __shared.<name>`. Resolution happens at render time, not parse time.
    #[serde(default)]
    pub shared_templates: HashMap<String, String>,
}

// ---------------------------------------------------------------------------
// Entry — how the runtime acquires issues to dispatch (poll / webhook / manual)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct EntryConfig {
    /// Active modes — runtime may enable multiple in the same process.
    pub modes: Vec<EntryMode>,
    #[serde(default)]
    pub poll: Option<PollConfig>,
    #[serde(default)]
    pub webhook: Option<WebhookConfig>,
    #[serde(default)]
    pub manual: Option<ManualConfig>,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum EntryMode {
    Poll,
    Webhook,
    Manual,
}

#[derive(Debug, Deserialize)]
pub struct PollConfig {
    pub interval_sec: u64,
    pub max_in_flight: usize,
    /// When absent the engine discovers repos from the app's sidebar cache
    /// (repos table) and verifies each has a local git clone at
    /// `{local_base_path}/{repoName}`. When present the command is still
    /// executed for backward compatibility.
    #[serde(default)]
    pub repo_source: Option<SourceCommand>,
    pub issue_source: SourceCommand,
}

#[derive(Debug, Deserialize)]
pub struct WebhookConfig {
    #[serde(default)]
    pub enabled: bool,
    pub listen: String,
    #[serde(default)]
    pub secret_env: Option<String>,
    #[serde(default)]
    pub events: Vec<String>,
    pub issue_source: SourceCommand,
}

#[derive(Debug, Deserialize)]
pub struct ManualConfig {
    pub issue_source: SourceCommand,
}

/// Wraps a shell command template. Variables are referenced as `{repo}` /
/// `{issue_number}` and substituted by `command::render_template`.
#[derive(Debug, Deserialize)]
pub struct SourceCommand {
    pub command: String,
}

// ---------------------------------------------------------------------------
// Roles
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct RoleConfig {
    /// Path to a system prompt file, resolved relative to the workflow YAML's
    /// own directory at load time.
    pub system_prompt_file: String,
    /// Extra dirs to expose to claude. Strings may contain `{repo}` /
    /// `{repo_path}`; substitution happens at spawn time, not parse time.
    #[serde(default)]
    pub add_dirs: Vec<String>,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub disallowed_tools: Vec<String>,
    #[serde(default)]
    pub json_schema_file: Option<String>,
    pub budget_usd: f64,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub needs_worktree: bool,
    /// Per-mode overrides — e.g. `mode_overrides.review.allowed_tools` swaps
    /// the build toolset for a read-only review toolset on the same role.
    #[serde(default)]
    pub mode_overrides: HashMap<String, ModeOverride>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ModeOverride {
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub disallowed_tools: Vec<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub budget_usd: Option<f64>,
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct DispatchConfig {
    pub rules: Vec<DispatchRule>,
}

#[derive(Debug, Deserialize)]
pub struct DispatchRule {
    pub when: Predicate,
    pub then: Directive,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Predicate {
    All { all: Vec<Predicate> },
    Any { any: Vec<Predicate> },
    Not { not: Box<Predicate> },
    Atom(AtomPredicate),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AtomPredicate {
    HasLabel(String),
    /// Prefix match — `matches_label: "status:"` is true if any label
    /// starts with `"status:"`.
    MatchesLabel(String),
    NotHasLabel(String),
    /// `"open"` | `"closed"`.
    IssueState(String),
    HasMarker(String),
    /// Arbitrary minijinja boolean expression (e.g. `len(out.x) > 0`).
    Expr(String),

    // ---- pre-spawn-only atoms ------------------------------------------
    // These evaluate on a richer pre-spawn context (resolved role, repo
    // path, …). In a plain dispatch context they're meaningless and the
    // dispatcher returns `false` for them.
    /// True if the spawn's `{repo_path}` resolves to an existing directory.
    /// The bool is just a marker — value is always implicitly `true`.
    RepoPathExists(bool),
    /// True if the substituted path exists on disk.
    PathExists(String),
    /// Matches the resolved spawn role for this pre-spawn hook.
    Role(String),
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "snake_case", tag = "directive")]
pub enum Directive {
    NoAction {
        #[serde(default)]
        reason: String,
    },
    Wait {
        #[serde(default)]
        reason: String,
    },
    HumanReview {
        #[serde(default)]
        reason: String,
    },
    SpawnFresh {
        role: String,
        #[serde(default)]
        mode: Option<String>,
        #[serde(default)]
        reason: String,
    },
}

// ---------------------------------------------------------------------------
// Hooks (pre-spawn / on-result / on-no-structured-output / unblock)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct PreSpawnHook {
    #[serde(rename = "if")]
    pub condition: Predicate,
    #[serde(rename = "do")]
    pub steps: Vec<Step>,
}

/// One on-result branch: matches when the spawn output's `kind` field equals
/// `kind`. Stored under its parent role inside `Workflow::on_result`.
#[derive(Debug, Deserialize)]
pub struct KindHandler {
    pub kind: String,
    pub steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
pub struct StepBlock {
    pub steps: Vec<Step>,
}

#[derive(Debug, Deserialize, Default)]
pub struct UnblockConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Steps run on each issue that gets unblocked this pass.
    #[serde(default)]
    pub on_unblock: Vec<Step>,
}

// ---------------------------------------------------------------------------
// Steps + control flow
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Step {
    /// Order matters: `Control` is tried first because its variants are
    /// keyed on `if` / `for_each` / `bind`-as-map, none of which clash
    /// with action keys (`create_issue`, `comment`, …).
    Control(ControlFlow),
    Action(ActionStep),
}

#[derive(Debug, Deserialize)]
pub struct ActionStep {
    #[serde(flatten)]
    pub action: ActionInput,
    /// Sibling-key bind — `- create_issue: { … }\n  bind: my_var` captures
    /// the action's result into `bindings.my_var` for later steps.
    #[serde(default)]
    pub bind: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ControlFlow {
    If {
        #[serde(rename = "if")]
        condition: String,
        steps: Vec<Step>,
        #[serde(default)]
        elif: Vec<ElifBranch>,
        #[serde(default, rename = "else")]
        else_steps: Option<Vec<Step>>,
    },
    ForEach {
        #[serde(rename = "for_each")]
        iter_expr: String,
        #[serde(rename = "as")]
        var: String,
        steps: Vec<Step>,
    },
    Bind {
        bind: HashMap<String, String>,
    },
}

#[derive(Debug, Deserialize)]
pub struct ElifBranch {
    pub elif: String,
    pub steps: Vec<Step>,
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionInput {
    CreateIssue {
        title: String,
        #[serde(default)]
        labels: Vec<LabelEntry>,
        #[serde(default)]
        body_template: Option<String>,
        #[serde(default)]
        body: Option<String>,
        /// Either a list of expressions yielding issue numbers, or a single
        /// Jinja expression that yields an array at runtime.
        #[serde(default)]
        deps: Option<DepsValue>,
    },
    /// Comment scalar form (`comment: |\n  text`) **and** detailed form
    /// (`comment: { body | template }`) both supported via `CommentBody`.
    Comment(CommentBody),
    /// `add_labels: ["a", "b"]` — value is a YAML list, not a struct.
    AddLabels(Vec<String>),
    /// `remove_labels: ["a", "b"]` — same shape as `add_labels`.
    RemoveLabels(Vec<String>),
    TransitionStatus {
        #[serde(default)]
        from: Option<String>,
        to: String,
    },
    /// `close_issue: true` — closes the current issue. Used by terminal
    /// roles (advisors emit advice + close, implementer review emits
    /// verdict + close).
    CloseIssue(bool),
    /// Multi-key map of `<!-- key: val -->` body markers to set in one shot.
    SetBodyMarker(HashMap<String, String>),
    PushBranchAndPr {
        /// Branch to push (defaults to `out.branch` at runtime when None).
        #[serde(default)]
        branch: Option<String>,
        base: String,
        title: String,
        #[serde(default)]
        body_template: Option<String>,
        /// Bool literal **or** Jinja expression — runtime evaluates the
        /// expression form against the current binding context.
        #[serde(default)]
        closes_issue: BoolOrExpr,
        /// Inline post-merge note rendered into the PR body.
        #[serde(default)]
        post_merge_note: Option<String>,
    },
    RunCommand {
        argv: Vec<String>,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        stdin: Option<String>,
        #[serde(default)]
        bind_stdout: Option<String>,
        #[serde(default)]
        bind_exit: Option<String>,
    },
    /// Pre-spawn only. `abort_spawn: true` halts the pre-spawn pipeline so
    /// the actual claude subprocess never starts.
    AbortSpawn(bool),
    /// Pre-spawn only. Redirects the spawn to a different role/mode pair —
    /// e.g. `reroute: { role: rlm-modeler, mode: bootstrap }`.
    Reroute {
        role: String,
        #[serde(default)]
        mode: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum CommentBody {
    /// `comment: |\n  text` — scalar block. Treated as a Jinja template at
    /// render time (inline form is the common case in practice).
    Inline(String),
    Detailed {
        #[serde(default)]
        template: Option<String>,
        #[serde(default)]
        body: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum LabelEntry {
    Plain(String),
    /// Conditional label — `- if: "<expr>"\n  label: "<value>"`. Evaluated
    /// per-issue at runtime; absent when the predicate is falsy.
    Conditional {
        #[serde(rename = "if")]
        condition: String,
        label: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum DepsValue {
    /// Single Jinja expression that yields an array of issue numbers.
    Expr(String),
    /// Static list of expressions, each yielding one issue number.
    List(Vec<String>),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum BoolOrExpr {
    Bool(bool),
    Expr(String),
}

impl Default for BoolOrExpr {
    fn default() -> Self {
        BoolOrExpr::Bool(false)
    }
}

// ---------------------------------------------------------------------------
// Custom deserializers
// ---------------------------------------------------------------------------

/// Helper struct used only during `on_result` deserialization — the YAML
/// is a flat list of `{ role, when:{kind}, steps }` which we group into
/// `HashMap<role, Vec<KindHandler>>` for O(1) lookup at dispatch time.
#[derive(Deserialize)]
struct OnResultEntry {
    role: String,
    when: KindMatcher,
    steps: Vec<Step>,
}

#[derive(Deserialize)]
struct KindMatcher {
    kind: String,
}

fn deserialize_on_result<'de, D>(d: D) -> Result<HashMap<String, Vec<KindHandler>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let entries: Vec<OnResultEntry> = Vec::deserialize(d)?;
    let mut map: HashMap<String, Vec<KindHandler>> = HashMap::new();
    for e in entries {
        map.entry(e.role).or_default().push(KindHandler {
            kind: e.when.kind,
            steps: e.steps,
        });
    }
    Ok(map)
}

// ---------------------------------------------------------------------------
// Public conveniences
// ---------------------------------------------------------------------------

impl Workflow {
    /// Parse a YAML string into a `Workflow`. Convenience over
    /// `serde_yaml::from_str` for call sites.
    pub fn from_yaml(s: &str) -> Result<Self, serde_yaml::Error> {
        serde_yaml::from_str(s)
    }

    /// Look up the ordered list of `KindHandler`s for a given role. Returns
    /// `None` if the role has no on_result entries.
    pub fn on_result_for(&self, role: &str) -> Option<&[KindHandler]> {
        self.on_result.get(role).map(|v| v.as_slice())
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_entry() -> &'static str {
        // Reusable fixture — every test that doesn't care about entry needs
        // SOMETHING because Workflow.entry is required.
        r#"
entry:
  modes: [manual]
  manual:
    issue_source:
      command: "gh issue view {issue_number} --repo {repo} --json number,title,body,labels,state"
"#
    }

    #[test]
    fn parses_minimal_workflow() {
        let yaml = format!(
            r#"
version: 1
{}
roles:
  shape:
    system_prompt_file: prompts/shape.md
    budget_usd: 1.50
dispatch:
  rules:
    - when:
        has_label: needs-shape
      then:
        directive: spawn_fresh
        role: shape
        reason: "auto"
"#,
            minimal_entry()
        );
        let wf = Workflow::from_yaml(&yaml).unwrap();
        assert_eq!(wf.version, 1);
        assert_eq!(wf.roles.len(), 1);
        assert!((wf.roles["shape"].budget_usd - 1.50).abs() < 1e-9);
        assert_eq!(wf.dispatch.rules.len(), 1);
        assert_eq!(wf.entry.modes, vec![EntryMode::Manual]);
    }

    #[test]
    fn entry_parses_all_three_modes_and_source_commands() {
        let yaml = r#"
version: 1
entry:
  modes: [poll, webhook, manual]
  poll:
    interval_sec: 30
    max_in_flight: 4
    repo_source:
      command: "agent-sweet-home registry list --json"
    issue_source:
      command: "gh issue list --repo {repo} --state open --json number,title,body,labels,state --limit 100"
  webhook:
    enabled: false
    listen: "0.0.0.0:8787"
    secret_env: GITHUB_WEBHOOK_SECRET
    events: [issues, issue_comment, label]
    issue_source:
      command: "gh issue view {issue_number} --repo {repo} --json number,title,body,labels,state"
  manual:
    issue_source:
      command: "gh issue view {issue_number} --repo {repo} --json number,title,body,labels,state"
roles: {}
dispatch:
  rules: []
"#;
        let wf = Workflow::from_yaml(yaml).unwrap();
        assert_eq!(
            wf.entry.modes,
            vec![EntryMode::Poll, EntryMode::Webhook, EntryMode::Manual]
        );
        let poll = wf.entry.poll.as_ref().unwrap();
        assert_eq!(poll.interval_sec, 30);
        assert_eq!(poll.max_in_flight, 4);
        assert!(poll.repo_source.as_ref().unwrap().command.contains("registry list"));
        assert!(poll.issue_source.command.contains("{repo}"));

        let webhook = wf.entry.webhook.as_ref().unwrap();
        assert!(!webhook.enabled);
        assert_eq!(webhook.listen, "0.0.0.0:8787");
        assert_eq!(webhook.secret_env.as_deref(), Some("GITHUB_WEBHOOK_SECRET"));
        assert_eq!(webhook.events, vec!["issues", "issue_comment", "label"]);

        let manual = wf.entry.manual.as_ref().unwrap();
        assert!(manual.issue_source.command.contains("{issue_number}"));
    }

    #[test]
    fn on_result_groups_by_role() {
        let yaml = format!(
            r#"
version: 1
{}
roles: {{}}
dispatch:
  rules: []
on_result:
  - role: arch-shape
    when: {{ kind: decomposition }}
    steps:
      - add_labels: ["status:done"]
  - role: arch-shape
    when: {{ kind: needs_consultation }}
    steps:
      - add_labels: ["arch-state:awaiting-consultation"]
  - role: implementer
    when: {{ kind: pr_delivered }}
    steps:
      - add_labels: ["impl-state:awaiting-review"]
"#,
            minimal_entry()
        );
        let wf = Workflow::from_yaml(&yaml).unwrap();
        let arch = wf.on_result_for("arch-shape").unwrap();
        assert_eq!(arch.len(), 2);
        assert_eq!(arch[0].kind, "decomposition");
        assert_eq!(arch[1].kind, "needs_consultation");
        let imp = wf.on_result_for("implementer").unwrap();
        assert_eq!(imp.len(), 1);
        assert_eq!(imp[0].kind, "pr_delivered");
        assert!(wf.on_result_for("rlm-modeler").is_none());
    }

    #[test]
    fn parses_compound_predicates() {
        let yaml = format!(
            r#"
version: 1
{}
roles: {{}}
dispatch:
  rules:
    - when:
        all:
          - has_label: ready
          - not:
              matches_label: "status:"
          - any:
              - issue_state: open
              - expr: "len(out.x) > 0"
      then:
        directive: no_action
        reason: "compound"
"#,
            minimal_entry()
        );
        let _ = yaml;
        let wf = Workflow::from_yaml(&format!(
            r#"
version: 1
{}
roles: {{}}
dispatch:
  rules:
    - when:
        all:
          - has_label: ready
          - not:
              matches_label: "status:"
          - any:
              - issue_state: open
              - expr: "len(out.x) > 0"
      then:
        directive: no_action
        reason: "compound"
"#,
            minimal_entry()
        ))
        .unwrap();
        let rule = &wf.dispatch.rules[0];
        match &rule.when {
            Predicate::All { all } => {
                assert_eq!(all.len(), 3);
                assert!(matches!(
                    &all[0],
                    Predicate::Atom(AtomPredicate::HasLabel(s)) if s == "ready"
                ));
                assert!(matches!(&all[1], Predicate::Not { .. }));
                match &all[2] {
                    Predicate::Any { any } => assert_eq!(any.len(), 2),
                    other => panic!("expected Any, got {other:?}"),
                }
            }
            other => panic!("expected All, got {other:?}"),
        }
    }

    #[test]
    fn parses_steps_mixing_control_and_actions() {
        let yaml = format!(
            r#"
version: 1
{}
roles: {{}}
dispatch:
  rules: []
on_result:
  - role: shape
    when:
      kind: "design_doc"
    steps:
      - if: "len(out.child_tasks) > 0"
        steps:
          - for_each: "out.child_tasks"
            as: child
            steps:
              - create_issue:
                  title: "{{{{ child.title }}}}"
                  labels: ["needs-{{{{ child.role }}}}"]
                  body_template: "templates/child_issue.md"
                  deps:
                    - "{{{{ rlm_issue.number }}}}"
                bind: child_issue
              - bind:
                  child_number: "child_issue.number"
        else:
          - comment:
              body: "(no children)"
      - add_labels: ["status:done"]
      - transition_status:
          from: "in_progress"
          to: "done"
      - set_body_marker:
          spec_version: "{{{{ out.version }}}}"
          spec_pr: "{{{{ pr.number }}}}"
      - push_branch_and_pr:
          base: "main"
          title: "feat: {{{{ out.title }}}}"
          body_template: "templates/pr.md"
          closes_issue: true
        bind: pr
      - run_command:
          argv: ["echo", "{{{{ out.kind }}}}"]
          bind_stdout: "echo_out"
          bind_exit: "echo_rc"
"#,
            minimal_entry()
        );
        let wf = Workflow::from_yaml(&yaml).unwrap();
        let handler = &wf.on_result_for("shape").unwrap()[0];
        assert_eq!(handler.kind, "design_doc");
        assert_eq!(handler.steps.len(), 6);
        assert!(matches!(handler.steps[0], Step::Control(ControlFlow::If { .. })));
        match &handler.steps[4] {
            Step::Action(a) => {
                assert_eq!(a.bind.as_deref(), Some("pr"));
                assert!(matches!(a.action, ActionInput::PushBranchAndPr { .. }));
            }
            other => panic!("expected push_branch_and_pr action, got {other:?}"),
        }
        assert!(matches!(
            handler.steps[5],
            Step::Action(ActionStep {
                action: ActionInput::RunCommand { .. },
                ..
            })
        ));
    }

    #[test]
    fn directive_round_trips_each_variant() {
        let cases = [
            ("directive: no_action\nreason: x", "NoAction"),
            ("directive: wait\nreason: y", "Wait"),
            ("directive: human_review", "HumanReview"),
            ("directive: spawn_fresh\nrole: shape\nmode: full", "SpawnFresh"),
        ];
        for (yaml, label) in cases {
            let d: Directive = serde_yaml::from_str(yaml).unwrap();
            let got = match d {
                Directive::NoAction { .. } => "NoAction",
                Directive::Wait { .. } => "Wait",
                Directive::HumanReview { .. } => "HumanReview",
                Directive::SpawnFresh { .. } => "SpawnFresh",
            };
            assert_eq!(got, label, "yaml: {yaml}");
        }
    }

    #[test]
    fn pre_spawn_uses_if_do_aliases() {
        let yaml = format!(
            r#"
version: 1
{}
roles: {{}}
dispatch:
  rules: []
pre_spawn:
  - if:
      has_label: needs-build
    do:
      - bind:
          rerouted: "'shape'"
"#,
            minimal_entry()
        );
        let wf = Workflow::from_yaml(&yaml).unwrap();
        assert_eq!(wf.pre_spawn.len(), 1);
        match &wf.pre_spawn[0].condition {
            Predicate::Atom(AtomPredicate::HasLabel(s)) => assert_eq!(s, "needs-build"),
            other => panic!("expected has_label, got {other:?}"),
        }
        assert_eq!(wf.pre_spawn[0].steps.len(), 1);
    }

    #[test]
    fn unblock_pass_defaults_to_disabled() {
        let yaml = format!("version: 1\n{}\nroles: {{}}\ndispatch:\n  rules: []\n", minimal_entry());
        let wf = Workflow::from_yaml(&yaml).unwrap();
        assert!(!wf.unblock_pass.enabled);
        assert!(wf.unblock_pass.on_unblock.is_empty());
    }

    // ---- Phase 1.5 ------------------------------------------------------

    #[test]
    fn pre_spawn_atoms_repo_path_exists_and_path_exists_and_role() {
        let yaml = format!(
            r#"
version: 1
{}
roles: {{}}
dispatch:
  rules: []
pre_spawn:
  - if: {{ not: {{ repo_path_exists: true }} }}
    do: []
  - if:
      all:
        - role: "arch-shape"
        - not: {{ path_exists: "{{repo_path}}/project-rlm" }}
    do: []
"#,
            minimal_entry()
        );
        let wf = Workflow::from_yaml(&yaml).unwrap();
        assert_eq!(wf.pre_spawn.len(), 2);
        match &wf.pre_spawn[0].condition {
            Predicate::Not { not } => match not.as_ref() {
                Predicate::Atom(AtomPredicate::RepoPathExists(true)) => {}
                other => panic!("expected RepoPathExists(true), got {other:?}"),
            },
            other => panic!("expected Not, got {other:?}"),
        }
        match &wf.pre_spawn[1].condition {
            Predicate::All { all } => {
                assert_eq!(all.len(), 2);
                match &all[0] {
                    Predicate::Atom(AtomPredicate::Role(r)) => assert_eq!(r, "arch-shape"),
                    other => panic!("expected Role, got {other:?}"),
                }
                match &all[1] {
                    Predicate::Not { not } => match not.as_ref() {
                        Predicate::Atom(AtomPredicate::PathExists(p)) => {
                            assert_eq!(p, "{repo_path}/project-rlm")
                        }
                        other => panic!("expected PathExists, got {other:?}"),
                    },
                    other => panic!("expected Not, got {other:?}"),
                }
            }
            other => panic!("expected All, got {other:?}"),
        }
    }

    #[test]
    fn abort_spawn_and_reroute_actions() {
        let yaml = format!(
            r#"
version: 1
{}
roles: {{}}
dispatch:
  rules: []
pre_spawn:
  - if: {{ repo_path_exists: true }}
    do:
      - abort_spawn: true
      - reroute: {{ role: rlm-modeler, mode: bootstrap }}
"#,
            minimal_entry()
        );
        let wf = Workflow::from_yaml(&yaml).unwrap();
        let steps = &wf.pre_spawn[0].steps;
        assert_eq!(steps.len(), 2);
        match &steps[0] {
            Step::Action(ActionStep { action: ActionInput::AbortSpawn(true), .. }) => {}
            other => panic!("expected AbortSpawn(true), got {other:?}"),
        }
        match &steps[1] {
            Step::Action(ActionStep {
                action: ActionInput::Reroute { role, mode },
                ..
            }) => {
                assert_eq!(role, "rlm-modeler");
                assert_eq!(mode.as_deref(), Some("bootstrap"));
            }
            other => panic!("expected Reroute, got {other:?}"),
        }
    }

    #[test]
    fn comment_accepts_scalar_and_detailed_forms() {
        let yaml = format!(
            r#"
version: 1
{}
roles: {{}}
dispatch:
  rules: []
on_result:
  - role: shape
    when:
      kind: x
    steps:
      - comment: |
          hello {{{{ issue.number }}}}
      - comment:
          body: "literal body"
      - comment:
          template: "templates/foo.md"
"#,
            minimal_entry()
        );
        let wf = Workflow::from_yaml(&yaml).unwrap();
        let steps = &wf.on_result_for("shape").unwrap()[0].steps;
        assert_eq!(steps.len(), 3);
        match &steps[0] {
            Step::Action(ActionStep { action: ActionInput::Comment(CommentBody::Inline(s)), .. }) => {
                assert!(s.contains("hello"));
            }
            other => panic!("expected inline comment, got {other:?}"),
        }
        match &steps[1] {
            Step::Action(ActionStep {
                action: ActionInput::Comment(CommentBody::Detailed { body, template }),
                ..
            }) => {
                assert_eq!(body.as_deref(), Some("literal body"));
                assert!(template.is_none());
            }
            other => panic!("expected detailed comment with body, got {other:?}"),
        }
        match &steps[2] {
            Step::Action(ActionStep {
                action: ActionInput::Comment(CommentBody::Detailed { body, template }),
                ..
            }) => {
                assert!(body.is_none());
                assert_eq!(template.as_deref(), Some("templates/foo.md"));
            }
            other => panic!("expected detailed comment with template, got {other:?}"),
        }
    }

    #[test]
    fn create_issue_supports_conditional_labels_and_string_deps() {
        let yaml = format!(
            r#"
version: 1
{}
roles: {{}}
dispatch:
  rules: []
on_result:
  - role: shape
    when: {{ kind: decomposition }}
    steps:
      - create_issue:
          title: "child"
          labels:
            - "agent:implementer"
            - "{{{{ 'status:blocked' if blocked else 'status:ready' }}}}"
            - if: "task.severity is defined"
              label: "severity:{{{{ task.severity }}}}"
          deps: "{{{{ resolved_dep_numbers }}}}"
        bind: rlm_child
"#,
            minimal_entry()
        );
        let wf = Workflow::from_yaml(&yaml).unwrap();
        let steps = &wf.on_result_for("shape").unwrap()[0].steps;
        match &steps[0] {
            Step::Action(ActionStep {
                action: ActionInput::CreateIssue { labels, deps, .. },
                bind,
            }) => {
                assert_eq!(bind.as_deref(), Some("rlm_child"));
                assert_eq!(labels.len(), 3);
                assert!(matches!(&labels[0], LabelEntry::Plain(s) if s == "agent:implementer"));
                assert!(matches!(&labels[1], LabelEntry::Plain(_)));
                match &labels[2] {
                    LabelEntry::Conditional { condition, label } => {
                        assert_eq!(condition, "task.severity is defined");
                        assert!(label.contains("severity:"));
                    }
                    other => panic!("expected Conditional, got {other:?}"),
                }
                match deps {
                    Some(DepsValue::Expr(s)) => assert!(s.contains("resolved_dep_numbers")),
                    other => panic!("expected DepsValue::Expr, got {other:?}"),
                }
            }
            other => panic!("expected CreateIssue, got {other:?}"),
        }
    }

    #[test]
    fn set_body_marker_takes_arbitrary_multi_key_map() {
        let yaml = format!(
            r#"
version: 1
{}
roles: {{}}
dispatch:
  rules: []
on_result:
  - role: shape
    when: {{ kind: x }}
    steps:
      - set_body_marker:
          arch-advisors-spawned: "be,fe,ops"
          arch-consultation-brief: "{{{{ out.brief }}}}"
"#,
            minimal_entry()
        );
        let wf = Workflow::from_yaml(&yaml).unwrap();
        let steps = &wf.on_result_for("shape").unwrap()[0].steps;
        match &steps[0] {
            Step::Action(ActionStep {
                action: ActionInput::SetBodyMarker(map),
                ..
            }) => {
                assert_eq!(map.len(), 2);
                assert_eq!(map["arch-advisors-spawned"], "be,fe,ops");
                assert!(map["arch-consultation-brief"].contains("{{"));
            }
            other => panic!("expected SetBodyMarker, got {other:?}"),
        }
    }

    #[test]
    fn push_branch_and_pr_accepts_branch_and_expr_closes_issue() {
        let yaml = format!(
            r#"
version: 1
{}
roles: {{}}
dispatch:
  rules: []
on_result:
  - role: rlm-modeler
    when: {{ kind: rlm_changes_applied }}
    steps:
      - push_branch_and_pr:
          branch: "{{{{ out.branch }}}}"
          base: main
          title: "[bootstrap] {{{{ issue.title }}}}"
          body_template: __shared.delivery_pr_body
          closes_issue: "{{{{ not is_bootstrap }}}}"
          post_merge_note: |
            🌱 Bootstrap PR — does NOT close.
        bind: pr
"#,
            minimal_entry()
        );
        let wf = Workflow::from_yaml(&yaml).unwrap();
        let steps = &wf.on_result_for("rlm-modeler").unwrap()[0].steps;
        match &steps[0] {
            Step::Action(ActionStep {
                action: ActionInput::PushBranchAndPr {
                    branch,
                    base,
                    closes_issue,
                    post_merge_note,
                    ..
                },
                bind,
            }) => {
                assert_eq!(bind.as_deref(), Some("pr"));
                assert_eq!(branch.as_deref(), Some("{{ out.branch }}"));
                assert_eq!(base, "main");
                match closes_issue {
                    BoolOrExpr::Expr(s) => assert!(s.contains("is_bootstrap")),
                    BoolOrExpr::Bool(_) => panic!("expected Expr, got Bool"),
                }
                assert!(post_merge_note.as_deref().unwrap().contains("🌱"));
            }
            other => panic!("expected PushBranchAndPr, got {other:?}"),
        }
    }

    #[test]
    fn role_config_parses_mode_overrides() {
        let yaml = format!(
            r#"
version: 1
{}
roles:
  implementer:
    system_prompt_file: x.md
    allowed_tools: ["Read", "Edit", "Write"]
    budget_usd: 2.5
    needs_worktree: true
    mode_overrides:
      review:
        allowed_tools: ["Read", "Glob", "Grep"]
        disallowed_tools: ["Edit", "Write"]
dispatch:
  rules: []
"#,
            minimal_entry()
        );
        let wf = Workflow::from_yaml(&yaml).unwrap();
        let cfg = &wf.roles["implementer"];
        assert_eq!(cfg.allowed_tools.len(), 3);
        let review = &cfg.mode_overrides["review"];
        assert_eq!(review.allowed_tools, vec!["Read", "Glob", "Grep"]);
        assert_eq!(review.disallowed_tools, vec!["Edit", "Write"]);
    }

    #[test]
    fn shared_templates_section_is_captured() {
        let yaml = format!(
            r#"
version: 1
{}
roles: {{}}
dispatch:
  rules: []
shared_templates:
  delivery_pr_body: |
    Implements task #{{{{ issue.number }}}}
  bug_triage_note: "see #{{{{ issue.number }}}}"
"#,
            minimal_entry()
        );
        let wf = Workflow::from_yaml(&yaml).unwrap();
        assert_eq!(wf.shared_templates.len(), 2);
        assert!(wf.shared_templates["delivery_pr_body"].contains("Implements"));
    }

    #[test]
    fn unblock_pass_captures_on_unblock_steps() {
        let yaml = format!(
            r#"
version: 1
{}
roles: {{}}
dispatch:
  rules: []
unblock_pass:
  enabled: true
  on_unblock:
    - comment: |
        🔓 unblocked
    - remove_labels: ["status:blocked"]
    - add_labels: ["status:ready"]
"#,
            minimal_entry()
        );
        let wf = Workflow::from_yaml(&yaml).unwrap();
        assert!(wf.unblock_pass.enabled);
        assert_eq!(wf.unblock_pass.on_unblock.len(), 3);
        match &wf.unblock_pass.on_unblock[0] {
            Step::Action(ActionStep {
                action: ActionInput::Comment(CommentBody::Inline(s)),
                ..
            }) => assert!(s.contains("unblocked")),
            other => panic!("expected inline Comment, got {other:?}"),
        }
    }

    #[test]
    fn nested_elif_branches_round_trip() {
        // Real YAML uses a flat `if/steps/elif/steps/else` shape that is
        // invalid YAML (duplicate `steps:` keys) — serde_yaml rejects it.
        // We require nested elif: `elif: [{ elif: cond, steps: [...] }, ...]`.
        let yaml = format!(
            r#"
version: 1
{}
roles: {{}}
dispatch:
  rules: []
on_result:
  - role: implementer
    when: {{ kind: review_completed }}
    steps:
      - if: "out.verdict == 'approve'"
        steps:
          - transition_status: {{ to: "done" }}
        elif:
          - elif: "out.verdict == 'request_changes'"
            steps:
              - add_labels: ["human-review"]
        else:
          - transition_status: {{ to: "ready" }}
"#,
            minimal_entry()
        );
        let wf = Workflow::from_yaml(&yaml).unwrap();
        let step = &wf.on_result_for("implementer").unwrap()[0].steps[0];
        match step {
            Step::Control(ControlFlow::If { condition, steps, elif, else_steps }) => {
                assert_eq!(condition, "out.verdict == 'approve'");
                assert_eq!(steps.len(), 1);
                assert_eq!(elif.len(), 1);
                assert_eq!(elif[0].elif, "out.verdict == 'request_changes'");
                assert!(else_steps.is_some());
            }
            other => panic!("expected If, got {other:?}"),
        }
    }

    /// Integration: parse the production agent-team v2 workflow YAML.
    /// `#[ignore]`d so the default `cargo test` doesn't depend on a sibling
    /// repo. Run with `cargo test -- --ignored parses_real_agent_team_workflow`.
    #[test]
    #[ignore]
    fn parses_real_agent_team_workflow() {
        let home = std::env::var("HOME").expect("HOME not set");
        let path = format!("{home}/Projects/agent-team/agent-team-v2.workflow.yaml");
        let yaml = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {path}: {e}"));
        Workflow::from_yaml(&yaml)
            .unwrap_or_else(|e| panic!("parse failed: {e}"));
    }
}

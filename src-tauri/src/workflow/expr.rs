//! Thin wrapper around `minijinja` for evaluating workflow conditions and
//! rendering template strings (comment bodies, PR titles, issue body
//! markers, …). Phase-1 covers the engine itself plus the four predicate
//! helpers that `dispatch` needs.
//!
//! Custom helpers exposed to YAML expressions:
//! - `has_label(name)`           — true if the issue has the literal label
//! - `matches_label(prefix)`     — true if any label starts with `prefix`
//! - `not_has_label(name)`       — convenience inverse of `has_label`
//! - `has_marker(key)`           — true if the issue body has `<!-- key: … -->`
//!
//! `ExprContext` is a transient struct used only by Phase-1 dispatch; the
//! richer `ActionContext` arrives in Phase 2 and will plug in the same way.

use minijinja::value::{Object, Value, ValueKind};
use minijinja::{Environment, Error};
use std::collections::HashMap;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Custom Jinja filters used by agent-team-v2.workflow.yaml
// ---------------------------------------------------------------------------

/// `42 | formatdep` → `"#42"`. Used in comment templates that list deps.
fn filter_formatdep(value: Value) -> Result<String, Error> {
    if let Some(n) = value.as_i64() {
        return Ok(format!("#{n}"));
    }
    if let Some(s) = value.as_str() {
        let trimmed = s.trim_start_matches('#');
        return Ok(format!("#{trimmed}"));
    }
    Ok(format!("#{value}"))
}

/// `idx | lookup_iter_result_number` → number of `_iter_results[idx]`.
/// Returns `none` when the index is out of range or the entry is malformed.
/// Used by arch-shape's child-task fan-out to resolve `task.deps` (array
/// indices into prior siblings of the same `for_each` loop).
fn filter_lookup_iter_result_number(value: Value, state: &minijinja::State) -> Value {
    let Some(idx) = value.as_i64() else {
        return Value::from(());
    };
    let Some(iter_results) = state.lookup("_iter_results") else {
        return Value::from(());
    };
    let Ok(item) = iter_results.get_item(&Value::from(idx)) else {
        return Value::from(());
    };
    item.get_attr("number").unwrap_or(Value::from(()))
}

/// `'42' | asint` → 42. minijinja has `int(...)` but our YAML uses the
/// `asint` name (older convention from the supervisor port). Provided as a
/// thin alias.
fn filter_asint(value: Value) -> Result<i64, Error> {
    if let Some(n) = value.as_i64() {
        return Ok(n);
    }
    if let Some(s) = value.as_str() {
        if let Ok(n) = s.trim().parse::<i64>() {
            return Ok(n);
        }
    }
    Err(Error::new(
        minijinja::ErrorKind::InvalidOperation,
        format!("asint: not coercible: {value:?}"),
    ))
}

/// Snapshot of an issue surfaced to expressions. Phase-1 only — Phase 2
/// extends this with bindings + `out` (structured spawn output).
#[derive(Debug, Clone)]
pub struct IssueSnapshot {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub state: String,
    pub labels: Vec<String>,
    pub markers: HashMap<String, String>,
}

/// What gets handed to the engine on every eval.
#[derive(Debug, Clone, Default)]
pub struct ExprContext {
    pub issue: Option<IssueSnapshot>,
    pub bindings: HashMap<String, serde_json::Value>,
    pub out: Option<serde_json::Value>,
}

impl ExprContext {
    pub fn with_issue(issue: IssueSnapshot) -> Self {
        Self {
            issue: Some(issue),
            ..Default::default()
        }
    }
}

/// Wraps `IssueSnapshot` so minijinja can read fields and call methods.
#[derive(Debug)]
struct IssueObject(IssueSnapshot);

impl Object for IssueObject {
    fn get_value(self: &Arc<Self>, key: &Value) -> Option<Value> {
        let key = key.as_str()?;
        match key {
            "number" => Some(Value::from(self.0.number)),
            "title" => Some(Value::from(self.0.title.clone())),
            "body" => Some(Value::from(self.0.body.clone())),
            "state" => Some(Value::from(self.0.state.clone())),
            "labels" => Some(Value::from(self.0.labels.clone())),
            "markers" => {
                let m: HashMap<String, String> = self.0.markers.clone();
                Some(Value::from(m))
            }
            _ => None,
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ExprError {
    #[error("template error: {0}")]
    Template(#[from] Error),
    #[error("expression must evaluate to a boolean (got {0:?})")]
    NotBoolean(String),
}

pub struct ExprEngine {
    env: Environment<'static>,
}

impl Default for ExprEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl ExprEngine {
    pub fn new() -> Self {
        let mut env = Environment::new();
        // Chainable undefined: `{{ spawn.role }}` against a missing `spawn`
        // renders as empty instead of erroring. The workflow YAML leans on
        // `| default(...)` filters everywhere, which need a chainable
        // undefined to short-circuit cleanly.
        env.set_undefined_behavior(minijinja::UndefinedBehavior::Chainable);
        env.add_filter("formatdep", filter_formatdep);
        env.add_filter("lookup_iter_result_number", filter_lookup_iter_result_number);
        env.add_filter("asint", filter_asint);
        Self { env }
    }

    fn make_root(ctx: &ExprContext) -> Value {
        let mut root: HashMap<String, Value> = HashMap::new();
        if let Some(issue) = &ctx.issue {
            root.insert(
                "issue".into(),
                Value::from_object(IssueObject(issue.clone())),
            );
            // Convenience top-level alias `markers.foo` for the common case.
            root.insert(
                "markers".into(),
                Value::from(issue.markers.clone()),
            );
        }
        for (k, v) in &ctx.bindings {
            root.insert(k.clone(), serde_to_value(v));
        }
        if let Some(out) = &ctx.out {
            root.insert("out".into(), serde_to_value(out));
        }
        Value::from(root)
    }

    /// Evaluate an expression like `len(out.x) > 0` to a boolean.
    pub fn eval_bool(&self, expr: &str, ctx: &ExprContext) -> Result<bool, ExprError> {
        let value = self.eval_value(expr, ctx)?;
        if value.kind() == ValueKind::Bool {
            return Ok(value.is_true());
        }
        if value.kind() == ValueKind::String {
            if let Some(s) = value.as_str() {
                return match s.trim() {
                    "true" | "True" | "1" => Ok(true),
                    "false" | "False" | "0" | "" => Ok(false),
                    other => Err(ExprError::NotBoolean(other.to_string())),
                };
            }
        }
        Err(ExprError::NotBoolean(format!("{value:?}")))
    }

    /// Evaluate an expression and return its raw value (used by `for_each`,
    /// `bind`, `deps`, etc.).
    pub fn eval_value(&self, expr: &str, ctx: &ExprContext) -> Result<Value, ExprError> {
        let root = Self::make_root(ctx);
        let wrapped = format!("{{{{ {expr} }}}}");
        let tmpl = self.env.template_from_str(&wrapped)?;
        // For a single-expression template we can render to a string and
        // parse, but we want to keep the original Value (numbers stay
        // numbers, lists stay lists). minijinja exposes
        // `tmpl.eval_to_state(...)` for that, but its public surface gives
        // back a string; instead we use `Environment::compile_expression`.
        let expression = self.env.compile_expression(expr)?;
        let value = expression.eval(root)?;
        let _ = tmpl; // ensure parse succeeds for symmetry with render()
        Ok(value)
    }

    /// Render a template string (e.g. comment body) with the given context.
    pub fn render(&self, template: &str, ctx: &ExprContext) -> Result<String, ExprError> {
        let root = Self::make_root(ctx);
        let tmpl = self.env.template_from_str(template)?;
        let out = tmpl.render(root)?;
        Ok(out)
    }

    // -- predicate helpers used by dispatch.rs --------------------------------

    pub fn has_label(ctx: &ExprContext, name: &str) -> bool {
        ctx.issue
            .as_ref()
            .map(|i| i.labels.iter().any(|l| l == name))
            .unwrap_or(false)
    }

    pub fn matches_label(ctx: &ExprContext, prefix: &str) -> bool {
        ctx.issue
            .as_ref()
            .map(|i| i.labels.iter().any(|l| l.starts_with(prefix)))
            .unwrap_or(false)
    }

    pub fn issue_state_is(ctx: &ExprContext, state: &str) -> bool {
        ctx.issue
            .as_ref()
            .map(|i| i.state == state)
            .unwrap_or(false)
    }

    pub fn has_marker(ctx: &ExprContext, key: &str) -> bool {
        ctx.issue
            .as_ref()
            .map(|i| i.markers.contains_key(key))
            .unwrap_or(false)
    }
}

fn serde_to_value(v: &serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::from(()),
        serde_json::Value::Bool(b) => Value::from(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::from(i)
            } else if let Some(f) = n.as_f64() {
                Value::from(f)
            } else {
                Value::from(n.to_string())
            }
        }
        serde_json::Value::String(s) => Value::from(s.clone()),
        serde_json::Value::Array(arr) => {
            Value::from(arr.iter().map(serde_to_value).collect::<Vec<_>>())
        }
        serde_json::Value::Object(map) => {
            let mut out: HashMap<String, Value> = HashMap::new();
            for (k, vv) in map {
                out.insert(k.clone(), serde_to_value(vv));
            }
            Value::from(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn issue_with_labels(labels: &[&str]) -> IssueSnapshot {
        IssueSnapshot {
            number: 42,
            title: "demo".into(),
            body: "hello".into(),
            state: "open".into(),
            labels: labels.iter().map(|s| s.to_string()).collect(),
            markers: HashMap::new(),
        }
    }

    #[test]
    fn render_substitutes_issue_fields() {
        let engine = ExprEngine::new();
        let ctx = ExprContext::with_issue(issue_with_labels(&["needs-shape"]));
        let out = engine
            .render("issue #{{ issue.number }}: {{ issue.title }}", &ctx)
            .unwrap();
        assert_eq!(out, "issue #42: demo");
    }

    #[test]
    fn eval_bool_handles_comparisons_and_logic() {
        let engine = ExprEngine::new();
        let mut ctx = ExprContext::with_issue(issue_with_labels(&["a", "b"]));
        ctx.out = Some(json!({ "items": [1, 2, 3] }));

        assert!(engine.eval_bool("issue.number == 42", &ctx).unwrap());
        assert!(engine.eval_bool("out.items|length > 0", &ctx).unwrap());
        assert!(engine
            .eval_bool("issue.number == 42 and issue.state == 'open'", &ctx)
            .unwrap());
        assert!(!engine.eval_bool("issue.state == 'closed'", &ctx).unwrap());
    }

    #[test]
    fn eval_value_returns_array_for_for_each() {
        let engine = ExprEngine::new();
        let mut ctx = ExprContext::default();
        ctx.out = Some(json!({ "child_tasks": [{"role": "fe"}, {"role": "be"}] }));
        let v = engine.eval_value("out.child_tasks", &ctx).unwrap();
        let items: Vec<_> = v.try_iter().unwrap().collect();
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn predicate_helpers_for_dispatch() {
        let mut snap = issue_with_labels(&["status:in-progress", "kind:feature"]);
        snap.markers.insert("spec_version".into(), "1".into());
        let ctx = ExprContext::with_issue(snap);
        assert!(ExprEngine::has_label(&ctx, "kind:feature"));
        assert!(!ExprEngine::has_label(&ctx, "missing"));
        assert!(ExprEngine::matches_label(&ctx, "status:"));
        assert!(!ExprEngine::matches_label(&ctx, "kindx:"));
        assert!(ExprEngine::issue_state_is(&ctx, "open"));
        assert!(ExprEngine::has_marker(&ctx, "spec_version"));
    }

    #[test]
    fn eval_bool_rejects_non_boolean_results() {
        let engine = ExprEngine::new();
        let ctx = ExprContext::default();
        let err = engine.eval_bool("123.45", &ctx).unwrap_err();
        assert!(matches!(err, ExprError::NotBoolean(_)));
    }
}

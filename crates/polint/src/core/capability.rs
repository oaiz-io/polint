//! Capability declarations and support views for rule planning.
//!
//! Extracted from the core monolith without behaviour changes.

use super::lang::Language;
use serde::{Deserialize, Serialize};

/// Fact families a rule wants the host to provide.
///
/// Capabilities are declarative: they describe which analysis facts a rule
/// consumes without changing the `Rule` trait. The current host may harvest a
/// superset of facts for a language; capability names are still the public
/// contract a rule should declare and docs should not imply unavailable facts
/// are produced.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Capabilities {
    /// Needs source files and syntax-derived facts. Currently descriptive; adapters still run their standard fact harvesters.
    pub syntax: bool,
    /// Needs syntactic import facts.
    pub imports: bool,
    /// Needs setup-aware resolved import facts.
    pub resolved_imports: bool,
    /// Needs file/package/module relationship graph facts.
    pub module_graph: bool,
    /// Needs symbol identities and definition facts.
    pub symbols: bool,
    /// Needs symbol reference facts; this also requires symbol identities.
    pub references: bool,
    /// Preview policy-level semantic events. Direct call-event matching is syntax-backed and upgrades when deeper call facts are present.
    pub events: bool,
    /// Preview policy-level call queries. Bounded reachable-call checks are provider-backed.
    pub calls: bool,
    /// Preview policy-level control-flow queries. Same-function guard/lifecycle checks are provider-backed.
    pub control_flow: bool,
    /// Reserved for future control-flow graph facts. Branch obligations are available through [`Capabilities::branch_obligations`].
    pub cfg: bool,
    /// Needs resolved call edges between functions.
    pub call_graph: bool,
    /// Needs the typed Go frontend's facts: function signatures, parameters,
    /// struct fields, generic instantiations, method sets and interface
    /// satisfaction.
    pub go_types: bool,
    /// Requests `call_graph` optionally: the rule still runs, with no view,
    /// when the capability is unavailable.
    #[serde(default)]
    pub call_graph_optional: bool,
    /// Requests `go_types` optionally: the rule still runs, with no view, when
    /// the capability is unavailable.
    #[serde(default)]
    pub go_types_optional: bool,
    /// Preview policy-level data-flow queries. Bounded source/sink/barrier checks are provider-backed.
    pub dataflow: bool,
    /// Needs Go test facts harvested from `_test.go` files.
    pub go_tests: bool,
    /// Needs syntax-level branch obligation facts.
    pub branch_obligations: bool,
    /// Reserved for future external coverage imports.
    pub coverage_facts: bool,
    /// Needs aggregate-like Go test metrics currently stored on Go test facts.
    pub test_suite_metrics: bool,
    /// Needs derived source-file size and aggregate function metrics.
    pub file_metrics: bool,
    /// Needs derived per-function size metrics.
    pub function_metrics: bool,
    /// Needs derived per-function complexity metrics.
    pub complexity_metrics: bool,
    /// Needs TypeScript/JavaScript component-like function facts.
    pub ts_components: bool,
    /// Needs TypeScript/JavaScript class facts.
    pub ts_classes: bool,
    /// Needs string and regex literal facts.
    pub string_literals: bool,
    /// Needs JSX attribute facts.
    pub jsx_attributes: bool,
    /// Needs the `polint review` changeset (diff-to-target-ref facts).
    pub changeset: bool,
}

impl Capabilities {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn syntax(mut self) -> Self {
        self.syntax = true;
        self
    }

    pub fn imports(mut self) -> Self {
        self.imports = true;
        self
    }

    pub fn resolved_imports(mut self) -> Self {
        self.resolved_imports = true;
        self
    }

    pub fn module_graph(mut self) -> Self {
        self.module_graph = true;
        self
    }

    pub fn symbols(mut self) -> Self {
        self.symbols = true;
        self
    }

    pub fn references(mut self) -> Self {
        self.references = true;
        self.symbols = true;
        self
    }

    pub fn events(mut self) -> Self {
        self.events = true;
        self
    }

    pub fn calls(mut self) -> Self {
        self.calls = true;
        self
    }

    pub fn control_flow(mut self) -> Self {
        self.control_flow = true;
        self
    }

    pub fn cfg(mut self) -> Self {
        self.cfg = true;
        self
    }

    pub fn call_graph(mut self) -> Self {
        self.call_graph = true;
        self
    }

    pub fn go_types(mut self) -> Self {
        self.go_types = true;
        self
    }

    pub fn call_graph_optional(mut self) -> Self {
        self.call_graph = true;
        self.call_graph_optional = true;
        self
    }

    pub fn go_types_optional(mut self) -> Self {
        self.go_types = true;
        self.go_types_optional = true;
        self
    }

    /// The requested capabilities a rule runs without when they are unavailable.
    pub(crate) fn optional_names(self) -> impl Iterator<Item = &'static str> {
        [
            ("call_graph", self.call_graph_optional),
            ("go_types", self.go_types_optional),
        ]
        .into_iter()
        .filter_map(|(name, optional)| optional.then_some(name))
    }

    pub fn dataflow(mut self) -> Self {
        self.dataflow = true;
        self
    }

    pub fn go_tests(mut self) -> Self {
        self.go_tests = true;
        self
    }

    pub fn branch_obligations(mut self) -> Self {
        self.branch_obligations = true;
        self
    }

    pub fn coverage_facts(mut self) -> Self {
        self.coverage_facts = true;
        self
    }

    pub fn test_suite_metrics(mut self) -> Self {
        self.test_suite_metrics = true;
        self
    }

    pub fn file_metrics(mut self) -> Self {
        self.file_metrics = true;
        self
    }

    pub fn function_metrics(mut self) -> Self {
        self.function_metrics = true;
        self
    }

    pub fn complexity_metrics(mut self) -> Self {
        self.complexity_metrics = true;
        self
    }

    pub fn ts_components(mut self) -> Self {
        self.ts_components = true;
        self
    }

    pub fn ts_classes(mut self) -> Self {
        self.ts_classes = true;
        self
    }

    pub fn string_literals(mut self) -> Self {
        self.string_literals = true;
        self
    }

    pub fn jsx_attributes(mut self) -> Self {
        self.jsx_attributes = true;
        self
    }

    pub fn changeset(mut self) -> Self {
        self.changeset = true;
        self
    }

    pub(crate) fn requested_names(self) -> impl Iterator<Item = &'static str> {
        [
            ("syntax", self.syntax),
            ("imports", self.imports),
            ("resolved_imports", self.resolved_imports),
            ("module_graph", self.module_graph),
            ("symbols", self.symbols),
            ("references", self.references),
            ("events", self.events),
            ("calls", self.calls),
            ("control_flow", self.control_flow),
            ("cfg", self.cfg),
            ("call_graph", self.call_graph),
            ("go_types", self.go_types),
            ("dataflow", self.dataflow),
            ("go_tests", self.go_tests),
            ("branch_obligations", self.branch_obligations),
            ("coverage_facts", self.coverage_facts),
            ("test_suite_metrics", self.test_suite_metrics),
            ("file_metrics", self.file_metrics),
            ("function_metrics", self.function_metrics),
            ("complexity_metrics", self.complexity_metrics),
            ("ts_components", self.ts_components),
            ("ts_classes", self.ts_classes),
            ("string_literals", self.string_literals),
            ("jsx_attributes", self.jsx_attributes),
            ("changeset", self.changeset),
        ]
        .into_iter()
        .filter_map(|(name, requested)| requested.then_some(name))
    }
}

/// Support state for a requested capability in the resolved analysis plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum CapabilitySupportStatus {
    /// The host can provide this capability for the current plan.
    Supported,
    /// The capability is known but not implemented or not requestable yet.
    Unsupported,
    /// The capability is implemented but required local setup is missing.
    SetupMissing,
}

/// Read-only support information for one capability row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CapabilitySupport {
    /// Stable capability name, such as `imports` or `cfg`.
    pub capability: String,
    /// Language this support row applies to, if language-specific.
    pub language: Option<Language>,
    /// Current support status for the capability row.
    pub status: CapabilitySupportStatus,
    /// Rule IDs that requested the capability.
    pub rules: Vec<String>,
    /// Deterministic explanation for unsupported or setup-missing rows.
    pub reason: Option<String>,
    /// Actionable remediation hint, when available.
    pub hint: Option<String>,
    /// Repository docs path for more context, when available.
    pub docs_path: Option<String>,
}

/// Read-only capability support rows exposed to rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CapabilitySupportView {
    entries: Vec<CapabilitySupport>,
}

impl CapabilitySupportView {
    /// Returns an empty support view for compatibility paths that do not build a plan.
    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Creates a support view from deterministic support rows.
    pub fn new(entries: Vec<CapabilitySupport>) -> Self {
        Self { entries }
    }

    /// Returns support rows in deterministic plan order.
    pub fn entries(&self) -> &[CapabilitySupport] {
        &self.entries
    }

    /// Returns the first status for a capability name, if present.
    pub fn status_for(&self, capability: &str) -> Option<CapabilitySupportStatus> {
        self.entries
            .iter()
            .find(|entry| entry.capability == capability)
            .map(|entry| entry.status.clone())
    }
}

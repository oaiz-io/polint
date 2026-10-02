use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::ops::Deref;

use serde::Serialize;

use crate::analysis::access_paths::facts::AccessPathProjection;
use crate::analysis::aliases::facts::{AliasOperand, AliasPrecision, AliasStatus};
use crate::analysis::calls::validate::validate_calls;
use crate::analysis::cfg::validate::validate_cfg;
use crate::analysis::data_flow::validate::validate_output as validate_data_flow_output;
use crate::analysis::domains::validate::validate_abstract_domains;
use crate::analysis::entrypoints::validate::validate_entrypoints;
use crate::analysis::identity::validate::validate_identity;
use crate::analysis::ids::{MirBodyId, MirOpId, PlaceId, ValueFactId};
use crate::analysis::points_to::facts::{
    PointsToBudgetStatus, PointsToConstraintKind, PointsToPrecision, PointsToStatus,
};
use crate::analysis::refined_calls::validate::validate_refined_calls;
use crate::analysis::semantic_graph::validate::validate_semantic_graph;
use crate::analysis::summaries::validate::validate_summaries;
use crate::analysis::types::facts::{TypePrecision, TypeShape, TypeStatus, TypeSubject};
use crate::analysis::validate::validate_semantic_mir;
use crate::analysis::values::facts::{ValueKind, ValuePrecision, ValueStatus, ValueSubject};
use crate::analysis_kernel::{
    FactFamily, FactPrecision, FactRef, PrecisionCeiling, ProviderManifest, ValidationDowngrades,
};
use crate::core::{
    AnalysisDb, BranchId, FileId, FunctionId, ImportId, ModuleNodeId, PackageId, ReferenceId,
    ResolvedImportId, Span, SymbolId,
};
use crate::diagnostics::{Diagnostic, Evidence, TextRange};
use crate::module_graph::topology::{
    DependencyRequirementId, ImportToPackageStatus, ResolvedDependencyKind, SourceSetId,
    TopologyPackageId, TopologyPrecision, TopologyStatus, WorkspaceRootId,
};
use crate::symbol_graph::semantic::{ExportId, ScopeId, SemanticStatus};

const SYMBOL_GRAPH_PROVIDER_ID: &str = "polint.symbol_graph";
const SEMANTIC_EVIDENCE_ORDER: (&str, &str, &str) = ("family", "stable_key", "reason");

/// Opt-in that forces whole-DB fact-metadata validation in release builds.
pub(crate) const VALIDATE_FACTS_ENV: &str = "POLINT_VALIDATE_FACTS";

#[cfg(test)]
static FACT_METADATA_VALIDATION_CALLS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Whether the kernel should run [`validate_fact_metadata`] on this process.
///
/// Debug builds validate by default so tests and local development keep the
/// assertion pass. Release production runs skip it unless `POLINT_VALIDATE_FACTS`
/// is set in the environment.
pub(crate) fn fact_metadata_validation_enabled() -> bool {
    fact_metadata_validation_enabled_for(
        cfg!(debug_assertions),
        std::env::var_os(VALIDATE_FACTS_ENV).is_some(),
    )
}

fn fact_metadata_validation_enabled_for(
    debug_assertions: bool,
    validate_facts_opt_in: bool,
) -> bool {
    debug_assertions || validate_facts_opt_in
}

#[cfg(test)]
pub(crate) fn fact_metadata_validation_call_count_for_test() -> usize {
    FACT_METADATA_VALIDATION_CALLS.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg(test)]
mod fact_metadata_validation_gate {
    use super::fact_metadata_validation_enabled_for;

    #[test]
    fn skips_release_builds_without_opt_in() {
        assert!(!fact_metadata_validation_enabled_for(false, false));
    }

    #[test]
    fn runs_in_debug_builds_by_default() {
        assert!(fact_metadata_validation_enabled_for(true, false));
    }

    #[test]
    fn runs_when_opted_in_for_release() {
        assert!(fact_metadata_validation_enabled_for(false, true));
    }
}

#[derive(Clone, Copy, Debug)]
enum Attribution {
    #[cfg(test)]
    Global,
    Provider(&'static str),
    Family(FactFamily),
    /// A family-scoped issue about fact *identity* rather than fact content.
    ///
    /// Two facts can legitimately collapse to one stable key — TypeScript
    /// declaration merging emits one `Export` per declaration for a single
    /// merged entity. The facts themselves are present and usable, so the
    /// issue is reported but must not downgrade the producing provider: doing
    /// so would block every rule requesting that provider's capabilities and
    /// silently strip real findings from any repo using a merged declaration.
    FamilyIdentity(FactFamily),
    Fact(FactRef),
}

type PendingIssue = (Diagnostic, Attribution);

#[derive(Clone, Debug)]
pub(crate) struct ValidationIssue {
    presentation: Diagnostic,
    reason: String,
    evidence: Vec<Evidence>,
    pub(crate) fact_family: Option<FactFamily>,
    pub(crate) provider_ids: Vec<String>,
    /// Whether this issue should mark its providers as validation-rejected.
    /// Identity-only issues are reported without downgrading — see
    /// [`Attribution::FamilyIdentity`].
    downgrades_providers: bool,
}

impl ValidationIssue {
    fn from_pending(
        (mut presentation, attribution): PendingIssue,
        db: &AnalysisDb,
        manifests_by_id: &BTreeMap<&'static str, ProviderManifest>,
    ) -> Self {
        // An empty owner set escalates to a *global* downgrade in
        // `ValidationReport::downgrades`, which fails every provider and
        // therefore blocks every capability-requesting rule. That is only ever
        // correct for a genuinely unattributable issue, so family- and
        // fact-scoped issues resolve their owners from the producers that
        // actually emitted facts in that family before falling back.
        let (fact_family, owners) = match attribution {
            #[cfg(test)]
            Attribution::Global => (None, None),
            Attribution::Provider(id) => (None, Some(BTreeSet::from([id]))),
            Attribution::Family(family) | Attribution::FamilyIdentity(family) => {
                (Some(family), family_owners(db, family))
            }
            Attribution::Fact(reference) => (
                Some(reference.family),
                db.metadata_for(reference)
                    .map(|metadata| BTreeSet::from([metadata.producer_id, metadata.layer_id]))
                    .or_else(|| family_owners(db, reference.family)),
            ),
        };
        // Keep the owners that name a real provider rather than discarding the
        // whole set when one of them does not. Extension producers and unknown
        // producer ids are reported separately by `validate_metadata_providers`;
        // they must not erase an otherwise precise attribution.
        let provider_ids = owners
            .into_iter()
            .flatten()
            .filter(|id| manifests_by_id.contains_key(id))
            .map(str::to_string)
            .collect();
        let reason = std::mem::take(&mut presentation.message);
        let evidence = std::mem::take(&mut presentation.evidence);
        Self {
            presentation,
            reason,
            evidence,
            fact_family,
            provider_ids,
            downgrades_providers: !matches!(attribution, Attribution::FamilyIdentity(_)),
        }
    }

    fn render(&self) -> Diagnostic {
        let mut diagnostic = self.presentation.clone();
        diagnostic.message.clone_from(&self.reason);
        diagnostic.evidence.clone_from(&self.evidence);
        diagnostic
    }
}
/// Providers that emitted at least one fact in `family`, taken from the
/// metadata rows the facts carry. Returns `None` when the family has no
/// metadata at all, which is the only case that stays unattributable.
fn family_owners(db: &AnalysisDb, family: FactFamily) -> Option<BTreeSet<&'static str>> {
    let owners = db
        .fact_meta()
        .family_rows(family)
        .flat_map(|metadata| [metadata.producer_id, metadata.layer_id])
        .collect::<BTreeSet<_>>();
    (!owners.is_empty()).then_some(owners)
}

#[derive(Clone, Debug)]
pub(crate) struct ValidationReport {
    diagnostics: Vec<Diagnostic>,
    pub(crate) issues: Vec<ValidationIssue>,
}
impl ValidationReport {
    pub(crate) fn downgrades(&self) -> ValidationDowngrades {
        let mut downgrades = ValidationDowngrades::default();
        for issue in self
            .issues
            .iter()
            .filter(|issue| issue.downgrades_providers)
        {
            if issue.provider_ids.is_empty() {
                downgrades.mark_global();
            } else {
                downgrades.extend_provider_ids(issue.provider_ids.iter().cloned());
            }
        }
        downgrades
    }
}
impl Deref for ValidationReport {
    type Target = [Diagnostic];
    fn deref(&self) -> &Self::Target {
        &self.diagnostics
    }
}
pub(crate) fn validate_fact_metadata(
    db: &AnalysisDb,
    manifests: &[ProviderManifest],
) -> ValidationReport {
    #[cfg(test)]
    FACT_METADATA_VALIDATION_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut pending = Vec::new();
    let ids = IdSets::from_db(db);
    let manifests_by_id = manifests
        .iter()
        .map(|manifest| (manifest.id, *manifest))
        .collect::<BTreeMap<_, _>>();
    macro_rules! collect {
        ($provider:expr, $validate:path $(, $arg:expr)*) => {{
            let mut diagnostics = Vec::new();
            $validate($($arg,)* &mut diagnostics);
            pending.extend(
                diagnostics
                    .into_iter()
                    .map(|diagnostic| (diagnostic, Attribution::Provider($provider))),
            );
        }};
    }
    validate_missing_metadata(db, &mut pending);
    validate_stable_key_conflicts(db, &mut pending);
    validate_references(db, &ids, &mut pending);
    validate_spans(db, &ids.files, &mut pending);
    collect!("polint.symbol_graph", validate_semantic_index, db, &ids);
    collect!("polint.module_topology", validate_topology_facts, db, &ids);
    collect!("polint.semantic_mir", validate_semantic_mir, db);
    collect!("polint.cfg", validate_cfg, db);
    collect!("polint.calls", validate_calls, db);
    collect!("polint.identity", validate_identity, db);
    collect!("polint.abstract_domains", validate_abstract_domains, db);
    collect!("polint.direct_summaries", validate_summaries, db);
    collect!("polint.entrypoints", validate_entrypoints, db);
    collect!("polint.type_value_alias", validate_type_value_alias, db);
    collect!("polint.semantic_graph", validate_semantic_graph, db);
    collect!("polint.refined_calls", validate_refined_calls, db);
    collect!("polint.data_flow", validate_data_flow, db);
    validate_metadata_providers(db, &manifests_by_id, &mut pending);
    validate_precision_ceilings(db, &manifests_by_id, &mut pending);
    let mut rendered = pending
        .into_iter()
        .map(|pending| {
            let issue = ValidationIssue::from_pending(pending, db, &manifests_by_id);
            (issue.render(), issue)
        })
        .collect::<Vec<_>>();
    rendered.sort_by(|(left, left_issue), (right, right_issue)| {
        diagnostic_order(left, right)
            .then_with(|| left_issue.fact_family.cmp(&right_issue.fact_family))
            .then_with(|| left_issue.provider_ids.cmp(&right_issue.provider_ids))
    });
    let (diagnostics, issues) = rendered.into_iter().unzip();
    ValidationReport {
        diagnostics,
        issues,
    }
}

fn validate_data_flow(db: &AnalysisDb, diagnostics: &mut Vec<Diagnostic>) {
    let output = crate::analysis::data_flow::store::DataFlowOutput {
        nodes: db.data_flow_nodes().to_vec(),
        edges: db.data_flow_edges().to_vec(),
        models: db.data_flow_models().to_vec(),
        budgets: db.data_flow_budgets().to_vec(),
    };
    for issue in validate_data_flow_output(&output, &db.stable_key_interner()) {
        diagnostics.push(internal_diagnostic(format!(
            "Data-flow validation issue for `{}`: {}",
            issue.stable_key_text, issue.reason
        )));
    }
}

fn validate_type_value_alias(db: &AnalysisDb, diagnostics: &mut Vec<Diagnostic>) {
    let interner = db.stable_key_interner();
    let type_value_alias_ids = TypeValueAliasIdSets::from_db(db);

    check_type_value_alias_stable_keys(
        diagnostics,
        FactFamily::Type,
        &interner,
        db.type_facts().iter().map(|fact| fact.stable_key),
    );
    check_type_value_alias_stable_keys(
        diagnostics,
        FactFamily::Value,
        &interner,
        db.value_facts().iter().map(|fact| fact.stable_key),
    );
    check_type_value_alias_stable_keys(
        diagnostics,
        FactFamily::NarrowedType,
        &interner,
        db.narrowed_type_facts().iter().map(|fact| fact.stable_key),
    );
    check_type_value_alias_stable_keys(
        diagnostics,
        FactFamily::AllocationToken,
        &interner,
        db.allocation_tokens().iter().map(|fact| fact.stable_key),
    );
    check_type_value_alias_stable_keys(
        diagnostics,
        FactFamily::AccessPath,
        &interner,
        db.access_path_facts().iter().map(|fact| fact.stable_key),
    );
    check_type_value_alias_stable_keys(
        diagnostics,
        FactFamily::AliasAnswer,
        &interner,
        db.alias_answers().iter().map(|fact| fact.stable_key),
    );
    check_type_value_alias_stable_keys(
        diagnostics,
        FactFamily::PointsToConstraint,
        &interner,
        db.points_to_constraints()
            .iter()
            .map(|fact| fact.stable_key),
    );
    check_type_value_alias_stable_keys(
        diagnostics,
        FactFamily::PointsToSet,
        &interner,
        db.points_to_sets().iter().map(|fact| fact.stable_key),
    );

    for fact in db.type_facts() {
        let stable_key = interner.resolve(fact.stable_key);
        validate_type_subject(
            diagnostics,
            &type_value_alias_ids,
            stable_key.as_ref(),
            &fact.subject,
        );
        validate_optional_type_value_alias_ref(
            diagnostics,
            &type_value_alias_ids.files,
            FactFamily::Type,
            stable_key.as_ref(),
            "file",
            fact.file,
        );
        validate_optional_type_value_alias_ref(
            diagnostics,
            &type_value_alias_ids.functions,
            FactFamily::Type,
            stable_key.as_ref(),
            "function",
            fact.function,
        );
        validate_optional_type_value_alias_ref(
            diagnostics,
            &type_value_alias_ids.bodies,
            FactFamily::Type,
            stable_key.as_ref(),
            "body",
            fact.body,
        );
        validate_optional_type_value_alias_ref(
            diagnostics,
            &type_value_alias_ids.places,
            FactFamily::Type,
            stable_key.as_ref(),
            "place",
            fact.place,
        );
        validate_optional_type_value_alias_ref(
            diagnostics,
            &type_value_alias_ids.cfg_blocks,
            FactFamily::Type,
            stable_key.as_ref(),
            "cfg_block",
            fact.cfg_block,
        );
        validate_optional_type_value_alias_ref(
            diagnostics,
            &type_value_alias_ids.operations,
            FactFamily::Type,
            stable_key.as_ref(),
            "operation",
            fact.operation,
        );
        validate_type_shape_refs(
            diagnostics,
            &type_value_alias_ids,
            stable_key.as_ref(),
            &fact.shape,
        );
        validate_type_status_precision(
            diagnostics,
            stable_key.as_ref(),
            fact.status,
            fact.precision,
        );
    }

    for fact in db.narrowed_type_facts() {
        let stable_key = interner.resolve(fact.stable_key);
        validate_type_value_alias_ref(
            diagnostics,
            &type_value_alias_ids.places,
            FactFamily::NarrowedType,
            stable_key.as_ref(),
            "place",
            fact.place,
        );
        validate_type_value_alias_ref(
            diagnostics,
            &type_value_alias_ids.type_sets,
            FactFamily::NarrowedType,
            stable_key.as_ref(),
            "type_set",
            fact.type_set,
        );
        validate_optional_type_value_alias_ref(
            diagnostics,
            &type_value_alias_ids.cfg_blocks,
            FactFamily::NarrowedType,
            stable_key.as_ref(),
            "cfg_block",
            fact.cfg_block,
        );
        validate_optional_type_value_alias_ref(
            diagnostics,
            &type_value_alias_ids.operations,
            FactFamily::NarrowedType,
            stable_key.as_ref(),
            "operation",
            fact.operation,
        );
        validate_optional_type_value_alias_ref(
            diagnostics,
            &type_value_alias_ids.places,
            FactFamily::NarrowedType,
            stable_key.as_ref(),
            "predicate",
            fact.predicate,
        );
        validate_optional_type_value_alias_ref(
            diagnostics,
            &type_value_alias_ids.files,
            FactFamily::NarrowedType,
            stable_key.as_ref(),
            "file",
            fact.file,
        );
        validate_optional_type_value_alias_ref(
            diagnostics,
            &type_value_alias_ids.functions,
            FactFamily::NarrowedType,
            stable_key.as_ref(),
            "function",
            fact.function,
        );
        validate_optional_type_value_alias_ref(
            diagnostics,
            &type_value_alias_ids.bodies,
            FactFamily::NarrowedType,
            stable_key.as_ref(),
            "body",
            fact.body,
        );
        validate_type_status_precision(
            diagnostics,
            stable_key.as_ref(),
            fact.status,
            fact.precision,
        );
    }

    for fact in db.value_facts() {
        let stable_key = interner.resolve(fact.stable_key);
        validate_value_subject(
            diagnostics,
            &type_value_alias_ids,
            stable_key.as_ref(),
            &fact.subject,
        );
        validate_value_kind(
            diagnostics,
            &type_value_alias_ids,
            stable_key.as_ref(),
            &fact.kind,
        );
        validate_optional_type_value_alias_ref(
            diagnostics,
            &type_value_alias_ids.files,
            FactFamily::Value,
            stable_key.as_ref(),
            "file",
            fact.file,
        );
        validate_optional_type_value_alias_ref(
            diagnostics,
            &type_value_alias_ids.functions,
            FactFamily::Value,
            stable_key.as_ref(),
            "function",
            fact.function,
        );
        validate_optional_type_value_alias_ref(
            diagnostics,
            &type_value_alias_ids.bodies,
            FactFamily::Value,
            stable_key.as_ref(),
            "body",
            fact.body,
        );
        validate_value_status_precision(
            diagnostics,
            stable_key.as_ref(),
            fact.status,
            fact.precision,
        );
    }

    for fact in db.allocation_tokens() {
        let stable_key = interner.resolve(fact.stable_key);
        validate_optional_type_value_alias_ref(
            diagnostics,
            &type_value_alias_ids.files,
            FactFamily::AllocationToken,
            stable_key.as_ref(),
            "file",
            fact.file,
        );
        validate_optional_type_value_alias_ref(
            diagnostics,
            &type_value_alias_ids.functions,
            FactFamily::AllocationToken,
            stable_key.as_ref(),
            "function",
            fact.function,
        );
        validate_optional_type_value_alias_ref(
            diagnostics,
            &type_value_alias_ids.bodies,
            FactFamily::AllocationToken,
            stable_key.as_ref(),
            "body",
            fact.body,
        );
        validate_optional_type_value_alias_ref(
            diagnostics,
            &type_value_alias_ids.places,
            FactFamily::AllocationToken,
            stable_key.as_ref(),
            "source_place",
            fact.source_place,
        );
        validate_optional_type_value_alias_ref(
            diagnostics,
            &type_value_alias_ids.operations,
            FactFamily::AllocationToken,
            stable_key.as_ref(),
            "source_operation",
            fact.source_operation,
        );
        if let Some(span) = &fact.span
            && let Some(reason) =
                span_failure_reason(db, &type_value_alias_ids.files, fact.file, span)
        {
            diagnostics.push(type_value_alias_diagnostic(
                FactFamily::AllocationToken,
                stable_key.as_ref(),
                "span",
                reason,
            ));
        }
    }

    for fact in db.access_path_facts() {
        let stable_key = interner.resolve(fact.stable_key);
        validate_type_value_alias_ref(
            diagnostics,
            &type_value_alias_ids.places,
            FactFamily::AccessPath,
            stable_key.as_ref(),
            "base",
            fact.base,
        );
        validate_optional_type_value_alias_ref(
            diagnostics,
            &type_value_alias_ids.files,
            FactFamily::AccessPath,
            stable_key.as_ref(),
            "file",
            fact.file,
        );
        validate_optional_type_value_alias_ref(
            diagnostics,
            &type_value_alias_ids.functions,
            FactFamily::AccessPath,
            stable_key.as_ref(),
            "function",
            fact.function,
        );
        validate_optional_type_value_alias_ref(
            diagnostics,
            &type_value_alias_ids.bodies,
            FactFamily::AccessPath,
            stable_key.as_ref(),
            "body",
            fact.body,
        );
        if fact.depth != fact.projections.len() as u32 {
            diagnostics.push(type_value_alias_diagnostic(
                FactFamily::AccessPath,
                stable_key.as_ref(),
                "depth",
                "access_path_depth_mismatch",
            ));
        }
        for projection in &fact.projections {
            if let AccessPathProjection::CallReturn(call) = projection {
                validate_type_value_alias_ref(
                    diagnostics,
                    &type_value_alias_ids.call_sites,
                    FactFamily::AccessPath,
                    stable_key.as_ref(),
                    "projection.call_return",
                    *call,
                );
            }
        }
    }

    for fact in db.points_to_constraints() {
        let stable_key = interner.resolve(fact.stable_key);
        validate_points_to_constraint_kind(
            diagnostics,
            &type_value_alias_ids,
            stable_key.as_ref(),
            &fact.kind,
        );
        validate_points_to_status_precision(
            diagnostics,
            FactFamily::PointsToConstraint,
            stable_key.as_ref(),
            fact.status,
            fact.precision,
        );
    }

    for fact in db.points_to_sets() {
        let stable_key = interner.resolve(fact.stable_key);
        validate_points_to_var(
            diagnostics,
            &type_value_alias_ids,
            FactFamily::PointsToSet,
            stable_key.as_ref(),
            "variable",
            fact.variable,
        );
        if fact.status == PointsToStatus::BudgetExceeded
            && fact.budget != PointsToBudgetStatus::BudgetExceeded
        {
            diagnostics.push(type_value_alias_diagnostic(
                FactFamily::PointsToSet,
                stable_key.as_ref(),
                "budget",
                "budget_status_mismatch",
            ));
        }
        for object in &fact.objects {
            validate_type_value_alias_ref(
                diagnostics,
                &type_value_alias_ids.object_tokens,
                FactFamily::PointsToSet,
                stable_key.as_ref(),
                "object",
                *object,
            );
        }
        validate_points_to_status_precision(
            diagnostics,
            FactFamily::PointsToSet,
            stable_key.as_ref(),
            fact.status,
            fact.precision,
        );
    }

    for fact in db.alias_answers() {
        let stable_key = interner.resolve(fact.stable_key);
        validate_alias_operand(
            db,
            diagnostics,
            &type_value_alias_ids.places,
            &type_value_alias_ids.access_paths,
            stable_key.as_ref(),
            fact.left,
        );
        validate_alias_operand(
            db,
            diagnostics,
            &type_value_alias_ids.places,
            &type_value_alias_ids.access_paths,
            stable_key.as_ref(),
            fact.right,
        );
        if matches!(fact.status, AliasStatus::MustAlias | AliasStatus::NoAlias)
            && (fact.evidence.is_empty() || fact.precision == AliasPrecision::Unknown)
        {
            diagnostics.push(type_value_alias_diagnostic(
                FactFamily::AliasAnswer,
                stable_key.as_ref(),
                "evidence",
                "overconfident_alias_answer",
            ));
        }
    }
}

#[derive(Debug, Default)]
struct TypeValueAliasIdSets {
    files: BTreeSet<FileId>,
    functions: BTreeSet<FunctionId>,
    bodies: BTreeSet<MirBodyId>,
    operations: BTreeSet<MirOpId>,
    places: BTreeSet<PlaceId>,
    cfg_blocks: BTreeSet<crate::analysis::cfg::ids::BasicBlockId>,
    call_sites: BTreeSet<crate::analysis::ids::CallSiteId>,
    symbols: BTreeSet<SymbolId>,
    type_sets: BTreeSet<crate::analysis::ids::TypeSetId>,
    value_facts: BTreeSet<ValueFactId>,
    allocations: BTreeSet<crate::analysis::ids::AllocationTokenId>,
    access_paths: BTreeSet<crate::analysis::ids::AccessPathId>,
    pt_vars: BTreeSet<crate::analysis::ids::PtVarId>,
    object_tokens: BTreeSet<crate::analysis::ids::ObjectTokenId>,
}

impl TypeValueAliasIdSets {
    fn from_db(db: &AnalysisDb) -> Self {
        let mut ids = Self {
            files: db.files().iter().map(|fact| fact.id).collect(),
            functions: db.functions().iter().map(|fact| fact.id).collect(),
            bodies: db.mir_bodies().iter().map(|fact| fact.id).collect(),
            operations: db.mir_operations().iter().map(|fact| fact.id).collect(),
            places: db.mir_places().iter().map(|fact| fact.id).collect(),
            cfg_blocks: db.cfg_blocks().iter().map(|fact| fact.id).collect(),
            call_sites: db.call_sites().iter().map(|fact| fact.id).collect(),
            symbols: db.symbols().iter().map(|fact| fact.id).collect(),
            type_sets: db.type_facts().iter().map(|fact| fact.type_set).collect(),
            value_facts: db.value_facts().iter().map(|fact| fact.id).collect(),
            allocations: db.allocation_tokens().iter().map(|fact| fact.id).collect(),
            access_paths: db.access_path_facts().iter().map(|fact| fact.id).collect(),
            pt_vars: BTreeSet::new(),
            object_tokens: BTreeSet::new(),
        };
        ids.pt_vars.extend(
            ids.places
                .iter()
                .map(|place| crate::analysis::points_to::vars::place_var(*place)),
        );
        ids.pt_vars.extend(
            ids.operations
                .iter()
                .map(|operation| crate::analysis::points_to::vars::operation_var(*operation)),
        );
        ids.pt_vars.extend(
            ids.allocations
                .iter()
                .map(|allocation| crate::analysis::points_to::vars::allocation_var(*allocation)),
        );
        ids.pt_vars.extend(
            ids.access_paths
                .iter()
                .map(|path| crate::analysis::points_to::vars::access_path_var(*path)),
        );
        ids.object_tokens.extend(
            ids.allocations
                .iter()
                .map(|allocation| crate::analysis::points_to::vars::allocation_object(*allocation)),
        );
        ids.object_tokens
            .extend(db.value_facts().iter().filter_map(|fact| {
                matches!(
                    fact.kind,
                    ValueKind::FunctionObject | ValueKind::ClassObject | ValueKind::ModuleObject
                )
                .then_some(
                    crate::analysis::points_to::vars::abstract_value_object(fact.value),
                )
            }));
        ids.object_tokens.extend(
            ids.value_facts
                .iter()
                .map(|value| crate::analysis::points_to::vars::value_fact_object(*value)),
        );
        ids
    }
}

fn check_type_value_alias_stable_keys(
    diagnostics: &mut Vec<Diagnostic>,
    family: FactFamily,
    interner: &crate::core::StableKeyInterner,
    keys: impl Iterator<Item = crate::core::StableKeyId>,
) {
    let mut seen = BTreeSet::new();
    for key in keys {
        let text = interner.resolve(key);
        if !seen.insert(text.to_string()) {
            diagnostics.push(type_value_alias_diagnostic(
                family,
                text.as_ref(),
                "stable_key",
                "duplicate_stable_key",
            ));
        }
    }
}

fn validate_alias_operand(
    _db: &AnalysisDb,
    diagnostics: &mut Vec<Diagnostic>,
    places: &BTreeSet<crate::analysis::ids::PlaceId>,
    access_paths: &BTreeSet<crate::analysis::ids::AccessPathId>,
    stable_key: &str,
    operand: AliasOperand,
) {
    let valid = match operand {
        AliasOperand::Place(place) => places.contains(&place),
        AliasOperand::AccessPath(path) => access_paths.contains(&path),
    };
    if !valid {
        diagnostics.push(type_value_alias_diagnostic(
            FactFamily::AliasAnswer,
            stable_key,
            "operand",
            "dangling_alias_operand",
        ));
    }
}

fn validate_type_subject(
    diagnostics: &mut Vec<Diagnostic>,
    ids: &TypeValueAliasIdSets,
    stable_key: &str,
    subject: &TypeSubject,
) {
    match subject {
        TypeSubject::Symbol(symbol) => validate_type_value_alias_ref(
            diagnostics,
            &ids.symbols,
            FactFamily::Type,
            stable_key,
            "subject.symbol",
            *symbol,
        ),
        TypeSubject::Place(place) => validate_type_value_alias_ref(
            diagnostics,
            &ids.places,
            FactFamily::Type,
            stable_key,
            "subject.place",
            *place,
        ),
        TypeSubject::Operation(operation) => validate_type_value_alias_ref(
            diagnostics,
            &ids.operations,
            FactFamily::Type,
            stable_key,
            "subject.operation",
            *operation,
        ),
        TypeSubject::Function(function) => validate_type_value_alias_ref(
            diagnostics,
            &ids.functions,
            FactFamily::Type,
            stable_key,
            "subject.function",
            *function,
        ),
        TypeSubject::Synthetic(_) | TypeSubject::Unknown(_) => {}
    }
}

fn validate_type_shape_refs(
    diagnostics: &mut Vec<Diagnostic>,
    ids: &TypeValueAliasIdSets,
    stable_key: &str,
    shape: &TypeShape,
) {
    if let TypeShape::Union(type_sets) | TypeShape::Intersection(type_sets) = shape {
        for type_set in type_sets {
            validate_type_value_alias_ref(
                diagnostics,
                &ids.type_sets,
                FactFamily::Type,
                stable_key,
                "shape.type_set",
                *type_set,
            );
        }
    }
}

fn validate_type_status_precision(
    diagnostics: &mut Vec<Diagnostic>,
    stable_key: &str,
    status: TypeStatus,
    precision: TypePrecision,
) {
    let invalid = match status {
        TypeStatus::Present => matches!(
            precision,
            TypePrecision::Unknown | TypePrecision::Unsupported
        ),
        TypeStatus::Unknown | TypeStatus::BudgetExceeded => {
            matches!(
                precision,
                TypePrecision::ExactLocal | TypePrecision::Unsupported
            )
        }
        TypeStatus::Unsupported => precision != TypePrecision::Unsupported,
        TypeStatus::SetupMissing => matches!(precision, TypePrecision::ExactLocal),
    };
    if invalid {
        diagnostics.push(type_value_alias_diagnostic(
            FactFamily::Type,
            stable_key,
            "status_precision",
            "type_status_precision_mismatch",
        ));
    }
}

fn validate_value_subject(
    diagnostics: &mut Vec<Diagnostic>,
    ids: &TypeValueAliasIdSets,
    stable_key: &str,
    subject: &ValueSubject,
) {
    match subject {
        ValueSubject::Place(place) => validate_type_value_alias_ref(
            diagnostics,
            &ids.places,
            FactFamily::Value,
            stable_key,
            "subject.place",
            *place,
        ),
        ValueSubject::Operation(operation) => validate_type_value_alias_ref(
            diagnostics,
            &ids.operations,
            FactFamily::Value,
            stable_key,
            "subject.operation",
            *operation,
        ),
        ValueSubject::Allocation(allocation) => validate_type_value_alias_ref(
            diagnostics,
            &ids.allocations,
            FactFamily::Value,
            stable_key,
            "subject.allocation",
            *allocation,
        ),
        ValueSubject::Synthetic(_) | ValueSubject::Unknown(_) => {}
    }
}

fn validate_value_kind(
    diagnostics: &mut Vec<Diagnostic>,
    ids: &TypeValueAliasIdSets,
    stable_key: &str,
    kind: &ValueKind,
) {
    match kind {
        ValueKind::PlaceRef(place) | ValueKind::CallReturn(place) => validate_type_value_alias_ref(
            diagnostics,
            &ids.places,
            FactFamily::Value,
            stable_key,
            "kind.place",
            *place,
        ),
        ValueKind::Object(allocation)
        | ValueKind::Array(allocation)
        | ValueKind::CompositeLiteral(allocation) => validate_type_value_alias_ref(
            diagnostics,
            &ids.allocations,
            FactFamily::Value,
            stable_key,
            "kind.allocation",
            *allocation,
        ),
        ValueKind::Null
        | ValueKind::Undefined
        | ValueKind::Nil
        | ValueKind::Bool(_)
        | ValueKind::Number(_)
        | ValueKind::String(_)
        | ValueKind::Literal(_)
        | ValueKind::FunctionObject
        | ValueKind::ClassObject
        | ValueKind::ModuleObject
        | ValueKind::Unknown { .. } => {}
    }
}

fn validate_value_status_precision(
    diagnostics: &mut Vec<Diagnostic>,
    stable_key: &str,
    status: ValueStatus,
    precision: ValuePrecision,
) {
    let invalid = match status {
        ValueStatus::Present => {
            matches!(
                precision,
                ValuePrecision::Unknown | ValuePrecision::Unsupported
            )
        }
        ValueStatus::Unknown | ValueStatus::BudgetExceeded => {
            matches!(
                precision,
                ValuePrecision::ExactLocal | ValuePrecision::Unsupported
            )
        }
        ValueStatus::Unsupported => precision != ValuePrecision::Unsupported,
        ValueStatus::SetupMissing => matches!(precision, ValuePrecision::ExactLocal),
    };
    if invalid {
        diagnostics.push(type_value_alias_diagnostic(
            FactFamily::Value,
            stable_key,
            "status_precision",
            "value_status_precision_mismatch",
        ));
    }
}

fn validate_points_to_constraint_kind(
    diagnostics: &mut Vec<Diagnostic>,
    ids: &TypeValueAliasIdSets,
    stable_key: &str,
    kind: &PointsToConstraintKind,
) {
    match kind {
        PointsToConstraintKind::AddressOf { dst, object } => {
            validate_points_to_var(
                diagnostics,
                ids,
                FactFamily::PointsToConstraint,
                stable_key,
                "kind.dst",
                *dst,
            );
            validate_type_value_alias_ref(
                diagnostics,
                &ids.object_tokens,
                FactFamily::PointsToConstraint,
                stable_key,
                "kind.object",
                *object,
            );
        }
        PointsToConstraintKind::CallReturn { dst, value } => {
            validate_points_to_var(
                diagnostics,
                ids,
                FactFamily::PointsToConstraint,
                stable_key,
                "kind.dst",
                *dst,
            );
            validate_type_value_alias_ref(
                diagnostics,
                &ids.value_facts,
                FactFamily::PointsToConstraint,
                stable_key,
                "kind.value",
                *value,
            );
        }
        PointsToConstraintKind::Copy { dst, src }
        | PointsToConstraintKind::SummaryFlow { dst, src, .. } => {
            validate_points_to_var(
                diagnostics,
                ids,
                FactFamily::PointsToConstraint,
                stable_key,
                "kind.dst",
                *dst,
            );
            validate_points_to_var(
                diagnostics,
                ids,
                FactFamily::PointsToConstraint,
                stable_key,
                "kind.src",
                *src,
            );
        }
        PointsToConstraintKind::Load { dst, pointer } => {
            validate_points_to_var(
                diagnostics,
                ids,
                FactFamily::PointsToConstraint,
                stable_key,
                "kind.dst",
                *dst,
            );
            validate_points_to_var(
                diagnostics,
                ids,
                FactFamily::PointsToConstraint,
                stable_key,
                "kind.pointer",
                *pointer,
            );
        }
        PointsToConstraintKind::Store { pointer, src } => {
            validate_points_to_var(
                diagnostics,
                ids,
                FactFamily::PointsToConstraint,
                stable_key,
                "kind.pointer",
                *pointer,
            );
            validate_points_to_var(
                diagnostics,
                ids,
                FactFamily::PointsToConstraint,
                stable_key,
                "kind.src",
                *src,
            );
        }
        PointsToConstraintKind::FieldLoad { dst, base, .. }
        | PointsToConstraintKind::ElementLoad { dst, base, .. } => {
            validate_points_to_var(
                diagnostics,
                ids,
                FactFamily::PointsToConstraint,
                stable_key,
                "kind.dst",
                *dst,
            );
            validate_points_to_var(
                diagnostics,
                ids,
                FactFamily::PointsToConstraint,
                stable_key,
                "kind.base",
                *base,
            );
        }
        PointsToConstraintKind::FieldStore { base, src, .. }
        | PointsToConstraintKind::ElementStore { base, src, .. } => {
            validate_points_to_var(
                diagnostics,
                ids,
                FactFamily::PointsToConstraint,
                stable_key,
                "kind.base",
                *base,
            );
            validate_points_to_var(
                diagnostics,
                ids,
                FactFamily::PointsToConstraint,
                stable_key,
                "kind.src",
                *src,
            );
        }
    }
}

fn validate_points_to_var(
    diagnostics: &mut Vec<Diagnostic>,
    ids: &TypeValueAliasIdSets,
    family: FactFamily,
    stable_key: &str,
    field: &'static str,
    value: crate::analysis::ids::PtVarId,
) {
    if ids.pt_vars.contains(&value)
        || crate::analysis::points_to::vars::is_solver_dynamic_var(value)
    {
        return;
    }
    diagnostics.push(type_value_alias_diagnostic(
        family,
        stable_key,
        field,
        "dangling_reference",
    ));
}

fn validate_points_to_status_precision(
    diagnostics: &mut Vec<Diagnostic>,
    family: FactFamily,
    stable_key: &str,
    status: PointsToStatus,
    precision: PointsToPrecision,
) {
    let invalid = match status {
        PointsToStatus::Present => {
            matches!(
                precision,
                PointsToPrecision::Unknown | PointsToPrecision::Unsupported
            )
        }
        PointsToStatus::Unknown | PointsToStatus::BudgetExceeded => {
            matches!(
                precision,
                PointsToPrecision::FlowInsensitive
                    | PointsToPrecision::LocalFlowSensitive
                    | PointsToPrecision::SummaryProjected
                    | PointsToPrecision::Unsupported
            )
        }
        PointsToStatus::Unsupported => precision != PointsToPrecision::Unsupported,
        PointsToStatus::SetupMissing => !matches!(precision, PointsToPrecision::Unknown),
    };
    if invalid {
        diagnostics.push(type_value_alias_diagnostic(
            family,
            stable_key,
            "status_precision",
            "points_to_status_precision_mismatch",
        ));
    }
}

fn validate_type_value_alias_ref<T>(
    diagnostics: &mut Vec<Diagnostic>,
    valid_ids: &BTreeSet<T>,
    family: FactFamily,
    stable_key: &str,
    field: &'static str,
    value: T,
) where
    T: Copy + Debug + Ord,
{
    if valid_ids.contains(&value) {
        return;
    }
    diagnostics.push(type_value_alias_diagnostic(
        family,
        stable_key,
        field,
        "dangling_reference",
    ));
}

fn validate_optional_type_value_alias_ref<T>(
    diagnostics: &mut Vec<Diagnostic>,
    valid_ids: &BTreeSet<T>,
    family: FactFamily,
    stable_key: &str,
    field: &'static str,
    value: Option<T>,
) where
    T: Copy + Debug + Ord,
{
    let Some(value) = value else {
        return;
    };
    validate_type_value_alias_ref(diagnostics, valid_ids, family, stable_key, field, value);
}

#[cfg(test)]
mod abstract_domains {
    use super::validate_fact_metadata;
    use crate::analysis::cfg::facts::{
        BasicBlockFact, BasicBlockKind, CfgFunctionFact, CfgNodeFact, CfgNodeKind, CfgPrecision,
        CfgStatus,
    };
    use crate::analysis::cfg::ids::{BasicBlockId, CfgFunctionId, CfgNodeId};
    use crate::analysis::cfg::store::CfgOutput;
    use crate::analysis::ids::{DomainEventId, DomainObservationId, MirBodyId, MirOpId, PlaceId};
    use crate::analysis::mir::body::{MirBody, MirOutput, MirStatus};
    use crate::analysis::mir::op::{AssignMode, MirOperation, MirOperationKind, MirValue};
    use crate::analysis::places::{PlaceFact, PlaceRoot, PlaceStatus};
    use crate::analysis_kernel::{
        AnalysisKernel, FactConfidence, FactFamily, FactMeta, FactPrecision, FactRef,
        ValidationStatus,
    };
    use crate::analysis_neutral::domains::facts::{
        DomainEventFact, DomainLocation, DomainObservationFact, DomainPrecision, DomainSlot,
        DomainStatus, DomainValue,
    };
    use crate::analysis_neutral::domains::store::DomainOutput;
    use crate::core::{AnalysisDb, FileId, FunctionFact, FunctionId, Language, Span};
    use std::path::PathBuf;

    #[test]
    fn abstract_domain_validation_reports_malformed_rows_with_generic_public_diagnostics() {
        let mut db = base_db();
        db.replace_abstract_domain_facts(DomainOutput {
            observations: vec![
                observation(
                    0,
                    "domain:dup",
                    DomainStatus::Present,
                    DomainValue::Label("nil".to_string()),
                ),
                DomainObservationFact {
                    id: DomainObservationId(1),
                    body: MirBodyId(99),
                    block: Some(BasicBlockId(99)),
                    operation: Some(MirOpId(99)),
                    place: Some(PlaceId(99)),
                    stable_key: crate::core::stable_key_for_test("domain:dup"),
                    ..observation(
                        1,
                        "domain:bad",
                        DomainStatus::Unknown,
                        DomainValue::Label("missing-top-reason".to_string()),
                    )
                },
            ],
            events: vec![DomainEventFact {
                id: DomainEventId(0),
                body: MirBodyId(99),
                block: Some(BasicBlockId(99)),
                operation: Some(MirOpId(99)),
                slot: Some(DomainSlot::Nilness),
                status: DomainStatus::BudgetExceeded,
                precision: DomainPrecision::Unknown,
                reason: "unknown_value".to_string(),
                stable_key: crate::core::stable_key_for_test("domain:event:bad"),
            }],
        });

        let diagnostics = validate_fact_metadata(&db, AnalysisKernel::provider_manifests());
        let domain = domain_diagnostics(&diagnostics);

        assert!(
            domain.len() >= 7,
            "expected abstract-domain validation diagnostics: {diagnostics:#?}"
        );
        // The user-facing rule id and message stay generic so malformed internal
        // rows never leak specifics into the public message; the per-row specifics
        // (family, stable_key, field, reason) are carried as structured evidence
        // for internal telemetry (mirrored by
        // `metadata_validation_conflict_records_render_internal_diagnostics_with_evidence`).
        assert!(domain.iter().all(|diagnostic| {
            diagnostic.rule_id == "polint/internal"
                && diagnostic.message == "Internal analysis validation failed."
                && !diagnostic.evidence.is_empty()
        }));
    }

    #[test]
    fn abstract_domain_validation_rejects_exact_metadata_precision() {
        let mut db = base_db();
        db.replace_abstract_domain_facts(DomainOutput {
            observations: vec![observation(
                0,
                "domain:exact-local-payload",
                DomainStatus::Present,
                DomainValue::Label("nil".to_string()),
            )],
            events: Vec::new(),
        });
        db.fact_meta_mut_for_test()
            .remove_for_test(FactRef::new(FactFamily::DomainObservation, 0));
        let stable_key = db
            .stable_key_interner()
            .intern("domain:exact-local-payload");
        db.fact_meta_mut_for_test().insert(
            FactRef::new(FactFamily::DomainObservation, 0),
            FactMeta {
                stable_key,
                producer_id: "polint.abstract_domains",
                layer_id: "polint.abstract_domains",
                precision: FactPrecision::Exact,
                confidence: FactConfidence::High,
                validation: ValidationStatus::NativeTrusted,
                payload_digest: "payload:exact-domain".to_string(),
            },
        );

        let diagnostics = validate_fact_metadata(&db, AnalysisKernel::provider_manifests());

        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic
                    .message
                    .starts_with("Fact metadata precision ceiling violated")
                    && diagnostic.evidence.iter().any(|evidence| {
                        evidence.label == "family" && evidence.value == "DomainObservation"
                    })
            }),
            "expected abstract-domain precision ceiling diagnostic: {diagnostics:#?}"
        );
    }

    fn base_db() -> AnalysisDb {
        let mut db = AnalysisDb::new();
        let interner = db.stable_key_interner();
        let file = db.add_file(
            PathBuf::from("src/app.ts"),
            "src/app.ts".to_string(),
            "export function app() { let value = 1; return value; }\n".to_string(),
        );
        db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file,
            "app".to_string(),
            span(file),
            Language::TypeScript,
            false,
            true,
            1,
            Vec::new(),
        ));
        db.replace_semantic_mir(MirOutput {
            bodies: vec![MirBody {
                id: MirBodyId(0),
                language: Language::TypeScript,
                file,
                function: FunctionId::from_raw(0),
                package: None,
                module: None,
                owner_stable_key: interner.intern("function:app".to_string()),
                span: span(file),
                stable_key: interner.intern("body:app".to_string()),
                status: MirStatus::Partial,
            }],
            places: vec![PlaceFact {
                id: PlaceId(0),
                language: Language::TypeScript,
                file: Some(file),
                function: Some(FunctionId::from_raw(0)),
                root: PlaceRoot::Local {
                    function: FunctionId::from_raw(0),
                    name: "value".to_string(),
                },
                projections: Vec::new(),
                stable_key: interner.intern("place:value".to_string()),
                status: PlaceStatus::Partial,
            }],
            operations: vec![MirOperation {
                id: MirOpId(0),
                body: MirBodyId(0),
                ordinal: 0,
                span: span(file),
                kind: MirOperationKind::Assign {
                    place: PlaceId(0),
                    value: MirValue::Literal {
                        value: "1".to_string(),
                    },
                    mode: AssignMode::DeclarationBinding,
                },
                stable_key: interner.intern("op:assign".to_string()),
                status: MirStatus::Partial,
            }],
            unsupported: Vec::new(),
            ..MirOutput::default()
        })
        .expect("semantic MIR rows should store");
        db.replace_cfg_facts(CfgOutput {
            functions: vec![CfgFunctionFact {
                id: CfgFunctionId(0),
                body: MirBodyId(0),
                function: FunctionId::from_raw(0),
                language: Language::TypeScript,
                file,
                span: span(file),
                entry_node: CfgNodeId(0),
                normal_exit_node: CfgNodeId(0),
                exceptional_exit_node: None,
                stable_key: interner.intern("cfg:function:app"),
                status: CfgStatus::Resolved,
                precision: CfgPrecision::ExactLowered,
            }],
            nodes: vec![CfgNodeFact {
                id: CfgNodeId(0),
                cfg_function: CfgFunctionId(0),
                body: MirBodyId(0),
                operation: Some(MirOpId(0)),
                block: BasicBlockId(0),
                kind: CfgNodeKind::Operation,
                span: Some(span(file)),
                generated: false,
                operation_ordinal: 0,
                stable_key: interner.intern("cfg:node:assign"),
                status: CfgStatus::Resolved,
                precision: CfgPrecision::ExactLowered,
            }],
            blocks: vec![BasicBlockFact {
                id: BasicBlockId(0),
                cfg_function: CfgFunctionId(0),
                kind: BasicBlockKind::StraightLine,
                first_node: Some(CfgNodeId(0)),
                last_node: Some(CfgNodeId(0)),
                reachable: true,
                reverse_postorder: 0,
                stable_key: interner.intern("cfg:block:app"),
                status: CfgStatus::Resolved,
                precision: CfgPrecision::ExactLowered,
            }],
            ..CfgOutput::empty()
        })
        .expect("cfg rows should store");
        db
    }

    fn observation(
        id: u64,
        stable_key: &str,
        status: DomainStatus,
        value: DomainValue,
    ) -> DomainObservationFact {
        DomainObservationFact {
            id: DomainObservationId(id),
            body: MirBodyId(0),
            block: Some(BasicBlockId(0)),
            operation: Some(MirOpId(0)),
            place: Some(PlaceId(0)),
            slot: DomainSlot::Nilness,
            location: DomainLocation::AfterOperation,
            value,
            status,
            precision: DomainPrecision::ExactLocal,
            stable_key: crate::core::stable_key_for_test(stable_key),
        }
    }

    fn span(file: FileId) -> Span {
        Span::new(file, 0, 10, 1, 1, 1, 11)
    }

    fn domain_diagnostics(
        diagnostics: &[crate::diagnostics::Diagnostic],
    ) -> Vec<&crate::diagnostics::Diagnostic> {
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.message == "Internal analysis validation failed.")
            .collect()
    }
}

#[cfg(test)]
mod type_value_alias_validation {
    use super::validate_fact_metadata;
    use crate::analysis::access_paths::facts::{
        AccessPathFact, AccessPathProjection, AccessPathStatus,
    };
    use crate::analysis::access_paths::store::AccessPathOutput;
    use crate::analysis::aliases::facts::{
        AliasAnswerFact, AliasOperand, AliasPrecision, AliasReason, AliasStatus,
    };
    use crate::analysis::aliases::store::AliasOutput;
    use crate::analysis::ids::{
        AbstractValueId, AccessPathId, AliasAnswerId, AllocationTokenId, MirOpId, ObjectTokenId,
        PlaceId, PointsToConstraintId, PointsToSetId, PtVarId, TypeFactId, TypeSetId, ValueFactId,
    };
    use crate::analysis::points_to::facts::{
        PointsToBudgetStatus, PointsToConstraintFact, PointsToConstraintKind, PointsToPrecision,
        PointsToSetFact, PointsToStatus,
    };
    use crate::analysis::points_to::store::PointsToOutput;
    use crate::analysis::types::facts::{
        NarrowedTypeFact, TypeConfidence, TypeFact, TypePhase, TypePrecision, TypeProvenance,
        TypeShape, TypeStatus, TypeSubject,
    };
    use crate::analysis::types::store::{TypeOutput, TypeValueAliasOutput};
    use crate::analysis::values::facts::{
        AllocationKind, AllocationTokenFact, ValueFact, ValueKind, ValuePrecision, ValueProvenance,
        ValueStatus, ValueSubject,
    };
    use crate::analysis::values::store::ValueOutput;
    use crate::analysis_kernel::AnalysisKernel;
    use crate::core::{AnalysisDb, Language, Span};

    #[test]
    fn type_value_alias_validation_reports_malformed_rows_deterministically() {
        let mut db = AnalysisDb::new();
        db.replace_type_value_alias_facts(TypeValueAliasOutput {
            types: TypeOutput {
                types: vec![
                    TypeFact {
                        shape: TypeShape::Union(vec![TypeSetId(99)]),
                        precision: TypePrecision::Unsupported,
                        ..type_fact(0, "type:dup", Some(PlaceId(99)))
                    },
                    type_fact(1, "type:dup", None),
                ],
                narrowed: vec![NarrowedTypeFact {
                    id: crate::analysis::ids::NarrowedTypeId(0),
                    place: PlaceId(77),
                    type_set: TypeSetId(88),
                    cfg_block: None,
                    operation: Some(MirOpId(99)),
                    predicate: Some(PlaceId(100)),
                    evidence: "bad".to_string(),
                    language: Language::TypeScript,
                    file: None,
                    function: None,
                    body: None,
                    precision: TypePrecision::ExactLocal,
                    status: TypeStatus::Unsupported,
                    stable_key: crate::core::stable_key_for_test("narrowed:bad"),
                }],
            },
            values: ValueOutput {
                values: vec![ValueFact {
                    id: ValueFactId(0),
                    subject: ValueSubject::Operation(MirOpId(77)),
                    value: AbstractValueId(0),
                    kind: ValueKind::Object(AllocationTokenId(99)),
                    language: Language::TypeScript,
                    file: None,
                    function: None,
                    body: None,
                    precision: ValuePrecision::Unknown,
                    status: ValueStatus::Present,
                    provenance: ValueProvenance::Native,
                    stable_key: crate::core::stable_key_for_test("value:bad"),
                }],
                allocations: vec![AllocationTokenFact {
                    id: AllocationTokenId(0),
                    kind: AllocationKind::ObjectLiteral,
                    language: Language::TypeScript,
                    file: None,
                    function: None,
                    body: None,
                    source_place: Some(PlaceId(55)),
                    source_operation: Some(MirOpId(56)),
                    span: Some(Span::new(
                        crate::core::FileId::from_raw(0),
                        10,
                        2,
                        2,
                        1,
                        1,
                        1,
                    )),
                    provenance: ValueProvenance::Native,
                    stable_key: crate::core::stable_key_for_test("allocation:bad"),
                }],
            },
            access_paths: AccessPathOutput {
                access_paths: vec![AccessPathFact {
                    id: AccessPathId(0),
                    base: PlaceId(42),
                    projections: vec![AccessPathProjection::CallReturn(
                        crate::analysis::ids::CallSiteId(99),
                    )],
                    depth: 2,
                    language: Language::TypeScript,
                    file: None,
                    function: None,
                    body: None,
                    status: AccessPathStatus::Resolved,
                    stable_key: crate::core::stable_key_for_test("path:bad"),
                }],
            },
            points_to: PointsToOutput {
                constraints: vec![
                    PointsToConstraintFact {
                        id: PointsToConstraintId(0),
                        kind: PointsToConstraintKind::AddressOf {
                            dst: PtVarId(1),
                            object: ObjectTokenId(99),
                        },
                        status: PointsToStatus::Present,
                        precision: PointsToPrecision::Unknown,
                        stable_key: crate::core::stable_key_for_test("pt:constraint:object"),
                    },
                    PointsToConstraintFact {
                        id: PointsToConstraintId(1),
                        kind: PointsToConstraintKind::CallReturn {
                            dst: PtVarId(1),
                            value: ValueFactId(99),
                        },
                        status: PointsToStatus::Present,
                        precision: PointsToPrecision::Unknown,
                        stable_key: crate::core::stable_key_for_test("pt:constraint:value"),
                    },
                ],
                sets: vec![PointsToSetFact {
                    id: PointsToSetId(0),
                    variable: PtVarId(1),
                    objects: vec![ObjectTokenId(1)],
                    status: PointsToStatus::BudgetExceeded,
                    precision: PointsToPrecision::Unknown,
                    budget: PointsToBudgetStatus::WithinBudget,
                    stable_key: crate::core::stable_key_for_test("pt:budget"),
                }],
            },
            aliases: AliasOutput {
                answers: vec![AliasAnswerFact {
                    id: AliasAnswerId(0),
                    left: AliasOperand::Place(PlaceId(1)),
                    right: AliasOperand::AccessPath(AccessPathId(99)),
                    status: AliasStatus::MustAlias,
                    reason: AliasReason::ExtensionProvided,
                    evidence: Vec::new(),
                    precision: AliasPrecision::Unknown,
                    stable_key: crate::core::stable_key_for_test("alias:bad"),
                }],
            },
        });

        let diagnostics = validate_fact_metadata(&db, AnalysisKernel::provider_manifests());
        let reasons = diagnostics
            .iter()
            .flat_map(|diagnostic| diagnostic.evidence.iter())
            .filter(|evidence| evidence.label == "reason")
            .map(|evidence| evidence.value.as_str())
            .collect::<std::collections::BTreeSet<_>>();

        for expected in [
            "duplicate_stable_key",
            "dangling_reference",
            "access_path_depth_mismatch",
            "span file does not exist",
            "type_status_precision_mismatch",
            "value_status_precision_mismatch",
            "points_to_status_precision_mismatch",
            "budget_status_mismatch",
            "dangling_alias_operand",
            "overconfident_alias_answer",
        ] {
            assert!(
                reasons.contains(expected),
                "missing {expected}: {diagnostics:#?}"
            );
        }
        let rendered = format!("{:#?}", &*diagnostics);
        for marker in [
            "polint.type_value_alias",
            "TypeFact",
            "NarrowedTypeFact",
            "ValueFact",
            "AllocationTokenFact",
            "AccessPathFact",
            "PointsToConstraintFact",
            "AliasAnswerFact",
            "PointsToSetFact",
            "Types<'_>",
            "Values<'_>",
            "Aliases<'_>",
            "points-to",
            "type_value_alias",
        ] {
            assert!(
                !rendered.contains(marker),
                "validation diagnostics should not leak `{marker}`: {rendered}"
            );
        }
    }

    #[test]
    fn type_value_alias_validation_accepts_value_derived_points_to_objects() {
        let mut db = AnalysisDb::new();
        db.replace_type_value_alias_facts(TypeValueAliasOutput {
            values: ValueOutput {
                values: vec![ValueFact {
                    id: ValueFactId(0),
                    subject: ValueSubject::Synthetic("call-result".to_string()),
                    value: AbstractValueId(0),
                    kind: ValueKind::Unknown {
                        evidence: "call return object token".to_string(),
                    },
                    language: Language::TypeScript,
                    file: None,
                    function: None,
                    body: None,
                    precision: ValuePrecision::Unknown,
                    status: ValueStatus::Unknown,
                    provenance: ValueProvenance::Generated,
                    stable_key: crate::core::stable_key_for_test("value:call-result"),
                }],
                allocations: Vec::new(),
            },
            points_to: PointsToOutput {
                constraints: Vec::new(),
                sets: vec![PointsToSetFact {
                    id: PointsToSetId(0),
                    variable: crate::analysis::points_to::vars::dynamic_var(0),
                    objects: vec![crate::analysis::points_to::vars::value_fact_object(
                        ValueFactId(0),
                    )],
                    status: PointsToStatus::Present,
                    precision: PointsToPrecision::FlowInsensitive,
                    budget: PointsToBudgetStatus::WithinBudget,
                    stable_key: crate::core::stable_key_for_test("points-to:value-object"),
                }],
            },
            ..TypeValueAliasOutput::default()
        });

        let diagnostics = validate_fact_metadata(&db, AnalysisKernel::provider_manifests());
        assert!(
            diagnostics.iter().all(|diagnostic| {
                !diagnostic
                    .evidence
                    .iter()
                    .any(|evidence| evidence.label == "component" && evidence.value == "analysis")
            }),
            "value-derived object tokens should validate: {diagnostics:#?}"
        );
    }

    fn type_fact(id: u64, stable_key: &str, place: Option<PlaceId>) -> TypeFact {
        TypeFact {
            id: TypeFactId(id),
            subject: place
                .map(TypeSubject::Place)
                .unwrap_or_else(|| TypeSubject::Synthetic(stable_key.to_string())),
            type_set: TypeSetId(id),
            shape: TypeShape::Unknown {
                reason: "fixture".to_string(),
            },
            phase: TypePhase::ExtensionProvided,
            language: Language::TypeScript,
            file: None,
            function: None,
            body: None,
            place,
            cfg_block: None,
            operation: None,
            precision: TypePrecision::Heuristic,
            confidence: TypeConfidence::Medium,
            status: TypeStatus::Present,
            provenance: TypeProvenance::Extension {
                extension_id: "fixture".to_string(),
            },
            stable_key: crate::core::stable_key_for_test(stable_key),
        }
    }
}

#[cfg(test)]
mod semantic_mir {
    use super::validate_fact_metadata;
    use crate::analysis::ids::{CallSiteId, MirBodyId, MirOpId, PlaceId, UnsupportedId};
    use crate::analysis::mir::body::{MirBody, MirOutput, MirStatus};
    use crate::analysis::mir::op::{
        ConservativeAction, MirOperation, MirOperationKind, MirValue, UnsupportedPrecision,
        UnsupportedSemanticFact,
    };
    use crate::analysis::places::{PlaceFact, PlaceProjection, PlaceRoot, PlaceStatus};
    use crate::analysis_kernel::{
        AnalysisKernel, FactConfidence, FactFamily, FactMeta, FactPrecision, FactRef,
        ValidationStatus,
    };
    use crate::core::{AnalysisDb, FileId, FunctionFact, FunctionId, Language, Span};
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    #[test]
    fn semantic_mir_validation_reports_malformed_rows_with_required_evidence() {
        let mut db = base_db();
        let interner = db.stable_key_interner();
        db.replace_semantic_mir(MirOutput {
            bodies: vec![
                body(
                    &interner,
                    0,
                    FunctionId::from_raw(0),
                    span(FileId::from_raw(0), 0, 20),
                    "body:dup",
                ),
                body(
                    &interner,
                    1,
                    FunctionId::from_raw(99),
                    span(FileId::from_raw(0), 0, 999),
                    "body:dup",
                ),
            ],
            places: vec![
                place(
                    &interner,
                    0,
                    FunctionId::from_raw(99),
                    vec![PlaceProjection::IndexUnknown {
                        evidence: String::new(),
                    }],
                    "place:bad",
                ),
                call_return_place(&interner, 1, "place:return"),
            ],
            operations: vec![MirOperation {
                id: MirOpId(0),
                body: MirBodyId(0),
                ordinal: 0,
                span: span(FileId::from_raw(0), 0, 20),
                kind: MirOperationKind::Call {
                    site: CallSiteId(0),
                    callee: MirValue::Unknown {
                        evidence: "dynamic".to_string(),
                    },
                    arguments: vec![PlaceId(0)],
                    return_place: PlaceId(1),
                },
                stable_key: interner.intern("op:call".to_string()),
                status: MirStatus::Partial,
            }],
            unsupported: vec![UnsupportedSemanticFact {
                id: UnsupportedId(0),
                body: Some(MirBodyId(0)),
                operation: Some(MirOpId(0)),
                language: Language::TypeScript,
                file: FileId::from_raw(0),
                span: span(FileId::from_raw(0), 0, 20),
                construct: String::new(),
                source_evidence: String::new(),
                affected_places: Vec::new(),
                affected_domains: Vec::new(),
                conservative_action: ConservativeAction::HavocAffectedPlaces,
                precision: UnsupportedPrecision::Unsupported,
                status: MirStatus::Unsupported,
                stable_key: interner.intern("unsupported:bad".to_string()),
            }],
            ..MirOutput::default()
        })
        .expect("semantic rows should store for validation");

        let diagnostics = validate_fact_metadata(&db, AnalysisKernel::provider_manifests());
        let semantic_mir = diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic
                    .message
                    .starts_with("Semantic MIR validation failed")
            })
            .collect::<Vec<_>>();

        assert!(
            semantic_mir.len() >= 5,
            "expected semantic MIR diagnostics: {diagnostics:#?}"
        );
        assert!(semantic_mir.iter().all(|diagnostic| {
            let labels = evidence_labels(diagnostic);
            labels.contains("family")
                && labels.contains("stable_key")
                && labels.contains("field")
                && labels.contains("reason")
        }));
    }

    #[test]
    fn semantic_mir_validation_rejects_exact_provider_precision() {
        let mut db = base_db();
        let interner = db.stable_key_interner();
        db.replace_semantic_mir(MirOutput {
            bodies: vec![body(
                &interner,
                0,
                FunctionId::from_raw(0),
                span(FileId::from_raw(0), 0, 20),
                "body:ok",
            )],
            places: Vec::new(),
            operations: Vec::new(),
            unsupported: Vec::new(),
            ..MirOutput::default()
        })
        .expect("semantic rows should store");
        db.fact_meta_mut_for_test()
            .remove_for_test(FactRef::new(FactFamily::MirBody, 0));
        let stable_key = db.stable_key_interner().intern("body:ok");
        db.fact_meta_mut_for_test().insert(
            FactRef::new(FactFamily::MirBody, 0),
            FactMeta {
                stable_key,
                producer_id: "polint.semantic_mir",
                layer_id: "polint.semantic_mir",
                precision: FactPrecision::Exact,
                confidence: FactConfidence::High,
                validation: ValidationStatus::NativeTrusted,
                payload_digest: "payload:exact-mir".to_string(),
            },
        );

        let diagnostics = validate_fact_metadata(&db, AnalysisKernel::provider_manifests());

        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic
                    .message
                    .starts_with("Semantic MIR validation failed")
                    && diagnostic.evidence.iter().any(|evidence| {
                        evidence.label == "reason" && evidence.value.contains("precision ceiling")
                    })
            }),
            "expected semantic MIR precision ceiling diagnostic: {diagnostics:#?}"
        );
    }

    fn base_db() -> AnalysisDb {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/app.ts"),
            "src/app.ts".to_string(),
            "export function app() { return 1; }\n".to_string(),
        );
        db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file,
            "app".to_string(),
            span(file, 0, 33),
            Language::TypeScript,
            false,
            true,
            1,
            Vec::new(),
        ));
        db
    }

    fn body(
        interner: &crate::core::StableKeyInterner,
        id: u64,
        function: FunctionId,
        span: Span,
        stable_key: &str,
    ) -> MirBody {
        MirBody {
            id: MirBodyId(id),
            language: Language::TypeScript,
            file: span.file,
            function,
            package: None,
            module: None,
            owner_stable_key: interner.intern(format!("function:{}", function.0)),
            span,
            stable_key: interner.intern(stable_key.to_string()),
            status: MirStatus::Partial,
        }
    }

    fn place(
        interner: &crate::core::StableKeyInterner,
        id: u64,
        function: FunctionId,
        projections: Vec<PlaceProjection>,
        stable_key: &str,
    ) -> PlaceFact {
        PlaceFact {
            id: PlaceId(id),
            language: Language::TypeScript,
            file: Some(FileId::from_raw(0)),
            function: Some(function),
            root: PlaceRoot::Local {
                function,
                name: "value".to_string(),
            },
            projections,
            stable_key: interner.intern(stable_key.to_string()),
            status: PlaceStatus::Partial,
        }
    }

    fn call_return_place(
        interner: &crate::core::StableKeyInterner,
        id: u64,
        stable_key: &str,
    ) -> PlaceFact {
        PlaceFact {
            id: PlaceId(id),
            language: Language::TypeScript,
            file: Some(FileId::from_raw(0)),
            function: Some(FunctionId::from_raw(0)),
            root: PlaceRoot::CallReturn {
                call: CallSiteId(0),
            },
            projections: Vec::new(),
            stable_key: interner.intern(stable_key.to_string()),
            status: PlaceStatus::Partial,
        }
    }

    fn span(file: FileId, start_byte: u32, end_byte: u32) -> Span {
        Span::new(
            file,
            start_byte,
            end_byte,
            1,
            start_byte + 1,
            1,
            end_byte + 1,
        )
    }

    fn evidence_labels(diagnostic: &crate::diagnostics::Diagnostic) -> BTreeSet<&str> {
        diagnostic
            .evidence
            .iter()
            .map(|evidence| evidence.label.as_str())
            .collect()
    }
}

#[cfg(test)]
mod cfg {
    use super::validate_fact_metadata;
    use crate::analysis::cfg::facts::{
        BasicBlockFact, BasicBlockKind, CfgEdgeFact, CfgEdgeKind, CfgFunctionFact, CfgNodeFact,
        CfgNodeKind, CfgPrecision, CfgStatus, CfgView,
    };
    use crate::analysis::cfg::ids::{BasicBlockId, CfgEdgeId, CfgFunctionId, CfgNodeId};
    use crate::analysis::cfg::store::CfgOutput;
    use crate::analysis::ids::MirBodyId;
    use crate::analysis_kernel::{
        AnalysisKernel, FactConfidence, FactFamily, FactMeta, FactPrecision, FactRef,
        ValidationStatus,
    };
    use crate::core::{AnalysisDb, FileId, FunctionFact, FunctionId, Language, Span};
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    #[test]
    fn cfg_validation_reports_malformed_rows_with_required_evidence() {
        let mut db = base_db();
        let interner = db.stable_key_interner();
        db.replace_cfg_facts(CfgOutput {
            functions: vec![
                function(
                    &interner,
                    "cfg:function:dup",
                    CfgFunctionId(0),
                    CfgNodeId(404),
                    CfgNodeId(405),
                ),
                function(
                    &interner,
                    "cfg:function:dup",
                    CfgFunctionId(1),
                    CfgNodeId(0),
                    CfgNodeId(1),
                ),
            ],
            nodes: vec![CfgNodeFact {
                id: CfgNodeId(0),
                cfg_function: CfgFunctionId(99),
                body: MirBodyId(99),
                operation: None,
                block: BasicBlockId(99),
                kind: CfgNodeKind::Operation,
                span: Some(Span::new(FileId::from_raw(0), 10, 1, 1, 11, 1, 2)),
                generated: false,
                operation_ordinal: 0,
                stable_key: interner.intern("cfg:node:bad"),
                status: CfgStatus::Resolved,
                precision: CfgPrecision::ExactLowered,
            }],
            blocks: vec![BasicBlockFact {
                id: BasicBlockId(0),
                cfg_function: CfgFunctionId(0),
                kind: BasicBlockKind::StraightLine,
                first_node: None,
                last_node: None,
                reachable: true,
                reverse_postorder: 0,
                stable_key: interner.intern("cfg:block:bad"),
                status: CfgStatus::Resolved,
                precision: CfgPrecision::ExactLowered,
            }],
            edges: vec![
                edge(
                    &interner,
                    "cfg:edge:one",
                    CfgEdgeId(0),
                    BasicBlockId(0),
                    BasicBlockId(1),
                ),
                edge(
                    &interner,
                    "cfg:edge:two",
                    CfgEdgeId(1),
                    BasicBlockId(0),
                    BasicBlockId(1),
                ),
            ],
            ..CfgOutput::empty()
        })
        .expect("cfg rows should store for validation");

        let diagnostics = validate_fact_metadata(&db, AnalysisKernel::provider_manifests());
        let cfg = cfg_diagnostics(&diagnostics);

        assert!(
            cfg.len() >= 6,
            "expected CFG validation diagnostics: {diagnostics:#?}"
        );
        assert!(cfg.iter().all(|diagnostic| {
            let labels = evidence_labels(diagnostic);
            labels.contains("family")
                && labels.contains("stable_key")
                && labels.contains("field")
                && labels.contains("reason")
        }));
    }

    #[test]
    fn cfg_validation_rejects_missing_exit_and_bad_reachability() {
        let mut db = base_db();
        let interner = db.stable_key_interner();
        db.replace_cfg_facts(CfgOutput {
            functions: vec![function(
                &interner,
                "cfg:function:shape",
                CfgFunctionId(0),
                CfgNodeId(0),
                CfgNodeId(1),
            )],
            nodes: vec![
                node(
                    &interner,
                    CfgNodeId(0),
                    BasicBlockId(0),
                    CfgNodeKind::Entry,
                    "cfg:node:entry",
                ),
                node(
                    &interner,
                    CfgNodeId(1),
                    BasicBlockId(1),
                    CfgNodeKind::Operation,
                    "cfg:node:body",
                ),
            ],
            blocks: vec![
                block(
                    &interner,
                    BasicBlockId(0),
                    BasicBlockKind::Entry,
                    CfgNodeId(0),
                    false,
                    "cfg:block:entry",
                ),
                block(
                    &interner,
                    BasicBlockId(1),
                    BasicBlockKind::StraightLine,
                    CfgNodeId(1),
                    true,
                    "cfg:block:body",
                ),
            ],
            edges: vec![
                edge(
                    &interner,
                    "cfg:edge:entry-body",
                    CfgEdgeId(0),
                    BasicBlockId(0),
                    BasicBlockId(1),
                ),
                edge(
                    &interner,
                    "cfg:edge:body-entry",
                    CfgEdgeId(1),
                    BasicBlockId(1),
                    BasicBlockId(0),
                ),
            ],
            ..CfgOutput::empty()
        })
        .expect("cfg rows should store for validation");

        let diagnostics = validate_fact_metadata(&db, AnalysisKernel::provider_manifests());

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.message.starts_with("CFG validation failed")
                && diagnostic.evidence.iter().any(|evidence| {
                    evidence.label == "reason" && evidence.value.contains("selected exit")
                })
        }));
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.message.starts_with("CFG validation failed")
                && diagnostic.evidence.iter().any(|evidence| {
                    evidence.label == "reason" && evidence.value.contains("graph reachability")
                })
        }));
    }

    #[test]
    fn cfg_validation_rejects_exact_provider_precision() {
        let mut db = base_db();
        let interner = db.stable_key_interner();
        db.replace_cfg_facts(CfgOutput {
            functions: vec![function(
                &interner,
                "cfg:function:ok",
                CfgFunctionId(0),
                CfgNodeId(0),
                CfgNodeId(1),
            )],
            ..CfgOutput::empty()
        })
        .expect("cfg rows should store");
        db.fact_meta_mut_for_test()
            .remove_for_test(FactRef::new(FactFamily::CfgFunction, 0));
        let stable_key = db.stable_key_interner().intern("cfg:function:ok");
        db.fact_meta_mut_for_test().insert(
            FactRef::new(FactFamily::CfgFunction, 0),
            FactMeta {
                stable_key,
                producer_id: "polint.cfg",
                layer_id: "polint.cfg",
                precision: FactPrecision::Exact,
                confidence: FactConfidence::High,
                validation: ValidationStatus::NativeTrusted,
                payload_digest: "payload:exact-cfg".to_string(),
            },
        );

        let diagnostics = validate_fact_metadata(&db, AnalysisKernel::provider_manifests());

        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic
                    .message
                    .starts_with("Fact metadata precision ceiling violated")
                    && diagnostic.evidence.iter().any(|evidence| {
                        evidence.label == "family" && evidence.value == "CfgFunction"
                    })
            }),
            "expected CFG precision ceiling diagnostic: {diagnostics:#?}"
        );
    }

    fn base_db() -> AnalysisDb {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/app.ts"),
            "src/app.ts".to_string(),
            "export function app() { return 1; }\n".to_string(),
        );
        db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file,
            "app".to_string(),
            span(file),
            Language::TypeScript,
            false,
            true,
            1,
            Vec::new(),
        ));
        db
    }

    fn function(
        interner: &crate::core::StableKeyInterner,
        stable_key: &str,
        id: CfgFunctionId,
        entry_node: CfgNodeId,
        normal_exit_node: CfgNodeId,
    ) -> CfgFunctionFact {
        CfgFunctionFact {
            id,
            body: MirBodyId(0),
            function: FunctionId::from_raw(0),
            language: Language::TypeScript,
            file: FileId::from_raw(0),
            span: span(FileId::from_raw(0)),
            entry_node,
            normal_exit_node,
            exceptional_exit_node: None,
            stable_key: interner.intern(stable_key),
            status: CfgStatus::Resolved,
            precision: CfgPrecision::ExactLowered,
        }
    }

    fn edge(
        interner: &crate::core::StableKeyInterner,
        stable_key: &str,
        id: CfgEdgeId,
        from_block: BasicBlockId,
        to_block: BasicBlockId,
    ) -> CfgEdgeFact {
        CfgEdgeFact {
            id,
            cfg_function: CfgFunctionId(0),
            view: CfgView::NormalControl,
            from: CfgNodeId(0),
            to: CfgNodeId(1),
            from_block,
            to_block,
            kind: CfgEdgeKind::Normal,
            label: None,
            stable_key: interner.intern(stable_key),
            status: CfgStatus::Resolved,
            precision: CfgPrecision::ExactLowered,
        }
    }

    fn node(
        interner: &crate::core::StableKeyInterner,
        id: CfgNodeId,
        block: BasicBlockId,
        kind: CfgNodeKind,
        stable_key: &str,
    ) -> CfgNodeFact {
        CfgNodeFact {
            id,
            cfg_function: CfgFunctionId(0),
            body: MirBodyId(0),
            operation: None,
            block,
            kind,
            span: Some(span(FileId::from_raw(0))),
            generated: true,
            operation_ordinal: 0,
            stable_key: interner.intern(stable_key),
            status: CfgStatus::Resolved,
            precision: CfgPrecision::ExactLowered,
        }
    }

    fn block(
        interner: &crate::core::StableKeyInterner,
        id: BasicBlockId,
        kind: BasicBlockKind,
        node: CfgNodeId,
        reachable: bool,
        stable_key: &str,
    ) -> BasicBlockFact {
        BasicBlockFact {
            id,
            cfg_function: CfgFunctionId(0),
            kind,
            first_node: Some(node),
            last_node: Some(node),
            reachable,
            reverse_postorder: id.0 as u32,
            stable_key: interner.intern(stable_key),
            status: CfgStatus::Resolved,
            precision: CfgPrecision::ExactLowered,
        }
    }

    fn span(file: FileId) -> Span {
        Span::new(file, 0, 10, 1, 1, 1, 11)
    }

    fn cfg_diagnostics(
        diagnostics: &[crate::diagnostics::Diagnostic],
    ) -> Vec<&crate::diagnostics::Diagnostic> {
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.message.starts_with("CFG validation failed"))
            .collect()
    }

    fn evidence_labels(diagnostic: &crate::diagnostics::Diagnostic) -> BTreeSet<&str> {
        diagnostic
            .evidence
            .iter()
            .map(|evidence| evidence.label.as_str())
            .collect()
    }
}

#[cfg(test)]
mod calls {
    use super::validate_fact_metadata;
    use crate::analysis::calls::facts::{
        CallAlgorithm, CallCallee, CallEdgeKind, CallPrecision, CallProvenance, CallSiteFact,
        CallSyntaxKind, CallTargetFact, CallTargetStatus, UnresolvedCallFact, UnresolvedCallReason,
    };
    use crate::analysis::calls::store::{CallOutput, CallStore};
    use crate::analysis::ids::{CallSiteId, CallTargetId, MirBodyId, MirOpId, PlaceId};
    use crate::analysis_kernel::{
        AnalysisKernel, FactConfidence, FactFamily, FactMeta, FactPrecision, FactRef,
        ValidationStatus,
    };
    use crate::core::{
        AnalysisDb, FileId, FunctionFact, FunctionId, Language, Span, SymbolFact, SymbolId,
        SymbolKind, SymbolNamespace, SymbolPrecision,
    };
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    #[test]
    fn calls_validation_reports_malformed_rows_with_required_evidence() {
        let mut db = base_db();
        db.replace_call_facts(CallOutput {
            sites: vec![
                site(0, "call-site:dup"),
                CallSiteFact {
                    id: CallSiteId(1),
                    file: FileId::from_raw(99),
                    caller: FunctionId::from_raw(99),
                    owner_symbol: Some(SymbolId::from_raw(99)),
                    body: MirBodyId(99),
                    operation: MirOpId(99),
                    span: Span::new(FileId::from_raw(0), 10, 1, 1, 11, 1, 2),
                    arguments: vec![PlaceId(99)],
                    receiver: Some(PlaceId(98)),
                    result: Some(PlaceId(97)),
                    stable_key: crate::core::StableKeyId(0),
                    ..site(1, "call-site:bad")
                },
            ],
            targets: vec![
                target(0, CallSiteId(0), "call-target:bad"),
                CallTargetFact {
                    id: CallTargetId(1),
                    status: CallTargetStatus::Resolved,
                    reason: Some(UnresolvedCallReason::DynamicProperty),
                    target_function: None,
                    target_symbol: None,
                    synthetic_target: None,
                    stable_key: crate::core::StableKeyId(1),
                    ..target(1, CallSiteId(0), "call-target:ok")
                },
                CallTargetFact {
                    id: CallTargetId(2),
                    status: CallTargetStatus::Unresolved,
                    target_function: None,
                    target_symbol: None,
                    synthetic_target: None,
                    stable_key: crate::core::StableKeyId(2),
                    ..target(2, CallSiteId(0), "call-target:ok")
                },
            ],
            unresolved: vec![UnresolvedCallFact {
                site: CallSiteId(0),
                caller: FunctionId::from_raw(99),
                status: CallTargetStatus::Resolved,
                reason: UnresolvedCallReason::Unknown,
                algorithm: CallAlgorithm::DirectReference,
                provenance: CallProvenance::Native,
                precision: CallPrecision::Exact,
                stable_key: crate::core::StableKeyId(3),
            }],
        })
        .expect("call rows should store for validation");

        let diagnostics = validate_fact_metadata(&db, AnalysisKernel::provider_manifests());
        let calls = call_diagnostics(&diagnostics);

        assert!(
            calls.len() >= 8,
            "expected call validation diagnostics: {diagnostics:#?}"
        );
        assert!(calls.iter().all(|diagnostic| {
            let labels = evidence_labels(diagnostic);
            labels.contains("family")
                && labels.contains("stable_key")
                && labels.contains("field")
                && labels.contains("reason")
        }));
        assert!(
            calls
                .iter()
                .any(|diagnostic| diagnostic.evidence.iter().any(|evidence| {
                    evidence.label == "reason" && evidence.value.contains("contradictory")
                })),
            "expected contradictory status diagnostic: {diagnostics:#?}"
        );
        assert!(
            calls
                .iter()
                .any(|diagnostic| diagnostic.evidence.iter().any(|evidence| {
                    evidence.label == "reason"
                        && evidence.value.contains("missing unresolved reason")
                })),
            "expected missing unresolved reason diagnostic: {diagnostics:#?}"
        );
    }

    #[test]
    fn calls_validation_rejects_unresolved_target_identity_with_reason() {
        let mut db = base_db();
        db.replace_call_facts(CallOutput {
            sites: vec![site(0, "call-site:ok")],
            targets: vec![CallTargetFact {
                status: CallTargetStatus::Unsupported,
                reason: Some(UnresolvedCallReason::FrameworkDispatch),
                target_function: Some(FunctionId::from_raw(1)),
                target_symbol: Some(SymbolId::from_raw(1)),
                synthetic_target: None,
                stable_key: crate::core::StableKeyId(1),
                ..target(0, CallSiteId(0), "call-target:ok")
            }],
            unresolved: Vec::new(),
        })
        .expect("call rows should store for validation");

        let diagnostics = validate_fact_metadata(&db, AnalysisKernel::provider_manifests());
        let calls = call_diagnostics(&diagnostics);

        assert!(
            calls.iter().any(|diagnostic| {
                diagnostic.evidence.iter().any(|evidence| {
                    evidence.label == "reason"
                        && evidence
                            .value
                            .contains("unresolved call target cannot carry target identity")
                })
            }),
            "expected contradictory target identity diagnostic: {diagnostics:#?}"
        );
    }

    #[test]
    fn calls_validation_rejects_exact_provider_precision() {
        let mut db = base_db();
        db.replace_call_facts(CallOutput {
            sites: vec![site(0, "call-site:ok")],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("call rows should store");
        db.fact_meta_mut_for_test()
            .remove_for_test(FactRef::new(FactFamily::CallSite, 0));
        let stable_key = db.stable_key_interner().intern("call-site:ok");
        db.fact_meta_mut_for_test().insert(
            FactRef::new(FactFamily::CallSite, 0),
            FactMeta {
                stable_key,
                producer_id: "polint.calls",
                layer_id: "polint.calls",
                precision: FactPrecision::Exact,
                confidence: FactConfidence::High,
                validation: ValidationStatus::NativeTrusted,
                payload_digest: "payload:exact-calls".to_string(),
            },
        );

        let diagnostics = validate_fact_metadata(&db, AnalysisKernel::provider_manifests());

        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic
                    .message
                    .starts_with("Fact metadata precision ceiling violated")
                    && diagnostic
                        .evidence
                        .iter()
                        .any(|evidence| evidence.label == "family" && evidence.value == "CallSite")
            }),
            "expected calls precision ceiling diagnostic: {diagnostics:#?}"
        );
    }

    #[test]
    fn calls_validation_exercises_all_d10_indexes() {
        let output = CallOutput {
            sites: vec![site(0, "call-site:ok")],
            targets: vec![target(0, CallSiteId(0), "call-target:ok")],
            unresolved: vec![unresolved(0, "call-unresolved:ok")],
        };
        let interner = crate::core::StableKeyInterner::default();
        let store =
            CallStore::from_output(output, &interner).expect("call store should index rows");

        assert_eq!(store.sites_by_caller(FunctionId::from_raw(0)).len(), 1);
        assert_eq!(store.targets_by_site(CallSiteId(0)).len(), 1);
        assert_eq!(store.outgoing_by_function(FunctionId::from_raw(0)).len(), 1);
        assert_eq!(store.outgoing_by_symbol(SymbolId::from_raw(0)).len(), 1);
        assert_eq!(store.incoming_by_symbol(SymbolId::from_raw(1)).len(), 1);
        assert_eq!(store.incoming_by_function(FunctionId::from_raw(1)).len(), 1);
        assert_eq!(
            store
                .unresolved_by_reason(UnresolvedCallReason::DynamicProperty)
                .len(),
            1
        );
        assert_eq!(
            store
                .unresolved_by_status(CallTargetStatus::Unresolved)
                .len(),
            1
        );

        let interner = crate::core::StableKeyInterner::default();
        let mut dangling = target(0, CallSiteId(99), "call-target:without-site");
        dangling.stable_key = interner.intern("call-target:without-site");
        let missing = CallStore::from_output(
            CallOutput {
                sites: Vec::new(),
                targets: vec![dangling],
                unresolved: Vec::new(),
            },
            &interner,
        )
        .expect_err("targets without sites should be rejected before indexing");
        assert!(missing.to_string().contains("dangling call site"));
    }

    fn base_db() -> AnalysisDb {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/app.ts"),
            "src/app.ts".to_string(),
            "export function app() { target(); }\n".to_string(),
        );
        db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file,
            "app".to_string(),
            span(file),
            Language::TypeScript,
            false,
            true,
            1,
            Vec::new(),
        ));
        db.push_function(FunctionFact::new(
            FunctionId::from_raw(1),
            file,
            "target".to_string(),
            span(file),
            Language::TypeScript,
            false,
            true,
            1,
            Vec::new(),
        ));
        let interner = db.stable_key_interner();
        db.replace_symbol_graph_facts(
            vec![
                symbol(&interner, SymbolId::from_raw(0), file, "app"),
                symbol(&interner, SymbolId::from_raw(1), file, "target"),
            ],
            Vec::new(),
            Vec::new(),
        );
        db
    }

    fn symbol(
        interner: &crate::core::StableKeyInterner,
        id: SymbolId,
        file: FileId,
        name: &str,
    ) -> SymbolFact {
        SymbolFact::new(
            id,
            Language::TypeScript,
            name.to_string(),
            name.to_string(),
            SymbolKind::Function,
            SymbolNamespace::Value,
            Some(file),
            None,
            None,
            None,
            Some(span(file)),
            true,
            interner.intern(format!("symbol:{name}")),
            SymbolPrecision::ExactLocal,
        )
    }

    fn site(id: u64, _stable_key: &str) -> CallSiteFact {
        CallSiteFact {
            in_throw: false,
            id: CallSiteId(id),
            language: Language::TypeScript,
            file: FileId::from_raw(0),
            caller: FunctionId::from_raw(0),
            owner_symbol: Some(SymbolId::from_raw(0)),
            body: MirBodyId(0),
            operation: MirOpId(0),
            span: span(FileId::from_raw(0)),
            kind: CallSyntaxKind::Function,
            callee: CallCallee::Identifier {
                reference: None,
                name: "target".to_string(),
            },
            receiver: None,
            arguments: Vec::new(),
            result: None,
            status: CallTargetStatus::Resolved,
            precision: CallPrecision::SetupAware,
            stable_key: crate::core::StableKeyId(id as u32),
        }
    }

    fn target(id: u64, site: CallSiteId, _stable_key: &str) -> CallTargetFact {
        CallTargetFact {
            id: CallTargetId(id),
            site,
            caller: FunctionId::from_raw(0),
            target_function: Some(FunctionId::from_raw(1)),
            target_symbol: Some(SymbolId::from_raw(1)),
            synthetic_target: None,
            edge_kind: CallEdgeKind::Direct,
            algorithm: CallAlgorithm::DirectReference,
            status: CallTargetStatus::Resolved,
            reason: None,
            provenance: CallProvenance::Native,
            precision: CallPrecision::SetupAware,
            stable_key: crate::core::StableKeyId(id as u32),
        }
    }

    fn unresolved(site: u64, _stable_key: &str) -> UnresolvedCallFact {
        UnresolvedCallFact {
            site: CallSiteId(site),
            caller: FunctionId::from_raw(0),
            status: CallTargetStatus::Unresolved,
            reason: UnresolvedCallReason::DynamicProperty,
            algorithm: CallAlgorithm::SyntaxOnly,
            provenance: CallProvenance::MirShape,
            precision: CallPrecision::Unknown,
            stable_key: crate::core::StableKeyId(site as u32),
        }
    }

    fn span(file: FileId) -> Span {
        Span::new(file, 0, 10, 1, 1, 1, 11)
    }

    fn call_diagnostics(
        diagnostics: &[crate::diagnostics::Diagnostic],
    ) -> Vec<&crate::diagnostics::Diagnostic> {
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.message.starts_with("Calls validation failed"))
            .collect()
    }

    fn evidence_labels(diagnostic: &crate::diagnostics::Diagnostic) -> BTreeSet<&str> {
        diagnostic
            .evidence
            .iter()
            .map(|evidence| evidence.label.as_str())
            .collect()
    }
}

#[cfg(test)]
mod semantic_index {
    use super::validate_fact_metadata;
    use crate::analysis_kernel::{
        AnalysisKernel, FactConfidence, FactFamily, FactMeta, FactPrecision, FactRef,
        ValidationStatus,
    };
    use crate::core::{
        AnalysisDb, FileId, Language, Span, SymbolFact, SymbolId, SymbolKind, SymbolNamespace,
        SymbolPrecision,
    };
    use crate::symbol_graph::semantic::{
        ExportFact, ExportId, ExportKind, GeneratedSymbolFact, GeneratedSymbolId,
        GeneratedSymbolKind, SemanticStatus, StableExportId, StableExportIdentity,
    };
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    #[test]
    fn semantic_validation_reports_malformed_generated_rows_with_evidence() {
        let mut db = semantic_db();
        db.replace_semantic_index_facts(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![GeneratedSymbolFact {
                id: GeneratedSymbolId(99),
                language: Language::TypeScript,
                file: Some(FileId::from_raw(404)),
                package: None,
                module: None,
                symbol_stable_key: db.stable_key_interner().intern("symbol:answer"),
                source_stable_key: db.stable_key_interner().intern(""),
                producer_id: String::new(),
                generator: "test".to_string(),
                generated_discriminator: String::new(),
                kind: GeneratedSymbolKind::BuildGenerated,
                span: Some(span(FileId::from_raw(404), 0, 999)),
                stable_key: db.stable_key_interner().intern("generated:bad"),
                status: SemanticStatus::Resolved,
            }],
            Vec::new(),
        );

        let diagnostics = validate_fact_metadata(&db, AnalysisKernel::provider_manifests());
        let semantic_diagnostics = diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic
                    .message
                    .starts_with("Semantic index validation failed")
            })
            .collect::<Vec<_>>();

        assert!(
            semantic_diagnostics.len() >= 4,
            "expected generated-row validation diagnostics: {diagnostics:#?}"
        );
        assert!(
            semantic_diagnostics
                .iter()
                .any(|diagnostic| diagnostic.rule_id == "polint/internal")
        );
        assert!(semantic_diagnostics.iter().all(|diagnostic| {
            let labels = evidence_labels(diagnostic);
            labels.contains("family") && labels.contains("stable_key") && labels.contains("reason")
        }));
    }

    #[test]
    fn semantic_validation_rejects_symbol_graph_exact_semantic_metadata() {
        let mut db = semantic_db();
        db.replace_semantic_index_facts(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![GeneratedSymbolFact {
                id: GeneratedSymbolId(0),
                language: Language::TypeScript,
                file: Some(FileId::from_raw(0)),
                package: None,
                module: None,
                symbol_stable_key: db.stable_key_interner().intern("symbol:answer"),
                source_stable_key: db.stable_key_interner().intern("symbol:answer"),
                producer_id: "polint.symbol_graph".to_string(),
                generator: "test".to_string(),
                generated_discriminator: "entrypoint".to_string(),
                kind: GeneratedSymbolKind::BuildGenerated,
                span: Some(span(FileId::from_raw(0), 0, 1)),
                stable_key: db.stable_key_interner().intern("generated:answer"),
                status: SemanticStatus::Generated,
            }],
            Vec::new(),
        );
        db.fact_meta_mut_for_test()
            .remove_for_test(FactRef::new(FactFamily::GeneratedSymbol, 0));
        let stable_key = db.stable_key_interner().intern("generated:answer");
        db.fact_meta_mut_for_test().insert(
            FactRef::new(FactFamily::GeneratedSymbol, 0),
            FactMeta {
                stable_key,
                producer_id: "polint.symbol_graph",
                layer_id: "polint.symbol_graph",
                precision: FactPrecision::Exact,
                confidence: FactConfidence::High,
                validation: ValidationStatus::NativeTrusted,
                payload_digest: "payload:exact-generated".to_string(),
            },
        );

        let diagnostics = validate_fact_metadata(&db, AnalysisKernel::provider_manifests());

        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic
                    .message
                    .starts_with("Semantic index validation failed")
                    && diagnostic.evidence.iter().any(|evidence| {
                        evidence.label == "reason"
                            && evidence.value.contains("provider precision ceiling")
                    })
                    && diagnostic.evidence.iter().any(|evidence| {
                        evidence.label == "family" && evidence.value == "GeneratedSymbol"
                    })
                    && diagnostic.evidence.iter().any(|evidence| {
                        evidence.label == "stable_key" && evidence.value == "generated:answer"
                    })
            }),
            "expected semantic precision ceiling diagnostic: {diagnostics:#?}"
        );
    }

    #[test]
    fn semantic_validation_rejects_missing_stable_export_symbol_key() {
        let mut db = semantic_db();
        db.replace_semantic_index_facts(
            Vec::new(),
            Vec::new(),
            vec![ExportFact {
                id: ExportId(0),
                language: Language::TypeScript,
                file: Some(FileId::from_raw(0)),
                package: None,
                module: None,
                scope: None,
                symbol: None,
                export_name: "answer".to_string(),
                namespace: SymbolNamespace::Value,
                kind: ExportKind::Named,
                stable_key: db.stable_key_interner().intern("export:answer"),
                status: SemanticStatus::Resolved,
            }],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![StableExportIdentity {
                id: StableExportId(0),
                export: ExportId(0),
                language: Language::TypeScript,
                package_key: None,
                module_key: Some("src/app.ts".to_string()),
                export_name: "answer".to_string(),
                namespace: SymbolNamespace::Value,
                symbol_stable_key: db.stable_key_interner().intern("symbol:missing"),
                generated_discriminator: Some("native".to_string()),
                stable_key: db.stable_key_interner().intern("stable-export:answer"),
                status: SemanticStatus::Resolved,
            }],
        );

        let diagnostics = validate_fact_metadata(&db, AnalysisKernel::provider_manifests());

        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic
                    .message
                    .starts_with("Semantic index validation failed")
                    && diagnostic.evidence.iter().any(|evidence| {
                        evidence.label == "reason"
                            && evidence
                                .value
                                .contains("StableExportIdentity.symbol_stable_key does not exist")
                    })
            }),
            "expected missing stable export symbol diagnostic: {diagnostics:#?}"
        );
    }

    fn semantic_db() -> AnalysisDb {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/app.ts"),
            "src/app.ts".to_string(),
            "export const answer = 1;\n".to_string(),
        );
        let interner = db.stable_key_interner();
        db.replace_symbol_graph_facts(
            vec![SymbolFact::new(
                SymbolId::from_raw(0),
                Language::TypeScript,
                "answer".to_string(),
                "answer".to_string(),
                SymbolKind::Constant,
                SymbolNamespace::Value,
                Some(file),
                None,
                None,
                None,
                Some(span(file, 13, 19)),
                true,
                interner.intern("symbol:answer".to_string()),
                SymbolPrecision::ExactLocal,
            )],
            Vec::new(),
            Vec::new(),
        );
        db
    }

    fn span(file: FileId, start_byte: u32, end_byte: u32) -> Span {
        Span::new(
            file,
            start_byte,
            end_byte,
            1,
            start_byte + 1,
            1,
            end_byte + 1,
        )
    }

    fn evidence_labels(diagnostic: &crate::diagnostics::Diagnostic) -> BTreeSet<&str> {
        diagnostic
            .evidence
            .iter()
            .map(|evidence| evidence.label.as_str())
            .collect()
    }
}

#[cfg(test)]
mod topology {
    use super::validate_fact_metadata;
    use crate::analysis_kernel::AnalysisKernel;
    use crate::core::{
        AnalysisDb, FileId, ImportFact, ImportId, Language, ModuleNodeId, ResolutionPrecision,
        ResolutionStatus, ResolvedImportFact, ResolvedImportId, Span,
    };
    use crate::module_graph::topology::{
        DependencyRequirementFact, DependencyRequirementId, ImportContextKind, ImportToPackageFact,
        ImportToPackageId, ImportToPackageStatus, RequirementKind, ResolvedDependencyEdgeFact,
        ResolvedDependencyEdgeId, ResolvedDependencyKind, SourceSetFact, SourceSetId,
        SourceSetKind, TopologyOutput, TopologyPackageFact, TopologyPackageId, TopologyPackageKind,
        TopologyPrecision, TopologyStatus, WorkspaceRootFact, WorkspaceRootId, WorkspaceRootKind,
    };
    use crate::symbol_graph::semantic::{
        SemanticImportFact, SemanticImportId, SemanticImportKind, SemanticStatus,
    };
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    #[test]
    fn topology_validation_reports_invalid_refs_and_malformed_paths() {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/app.ts"),
            "src/app.ts".to_string(),
            "import './target';\n".to_string(),
        );
        let import = db.push_import(ImportFact::new(
            ImportId::from_raw(99),
            file,
            None,
            "./target".to_string(),
            span(file),
            Language::TypeScript,
        ));
        db.replace_module_graph_facts(
            vec![ResolvedImportFact::new(
                ResolvedImportId::from_raw(0),
                import,
                file,
                None,
                ResolutionStatus::Unresolved,
                ResolutionPrecision::None,
                None,
            )],
            Vec::new(),
            Vec::new(),
        );
        db.replace_topology_facts(TopologyOutput {
            workspace_roots: vec![root("/absolute", "root:absolute")],
            packages: vec![package(
                "package:bad",
                Some(WorkspaceRootId(404)),
                Some(ModuleNodeId::from_raw(404)),
                "../escape",
                TopologyPrecision::ExactStatic,
                TopologyStatus::Present,
            )],
            source_sets: vec![SourceSetFact {
                id: SourceSetId(0),
                package: Some(TopologyPackageId(404)),
                root: Some(WorkspaceRootId(404)),
                kind: SourceSetKind::Source,
                path: r"src\app.ts".to_string(),
                language: Some(Language::TypeScript),
                files: vec![FileId::from_raw(404)],
                stable_key: crate::core::stable_key_for_test("source-set:bad"),
                producer_id: "test",
                precision: TopologyPrecision::ExactStatic,
                status: TopologyStatus::Present,
            }],
            import_to_package_edges: vec![import_edge(
                "import-to-package:bad",
                Some(ImportId::from_raw(404)),
                Some(ResolvedImportId::from_raw(404)),
                Some("semantic:missing".to_string()),
                Some(FileId::from_raw(404)),
                ImportToPackageStatus::Resolved,
                TopologyPrecision::ExactStatic,
            )],
            ..TopologyOutput::default()
        });

        let diagnostics = validate_fact_metadata(&db, AnalysisKernel::provider_manifests());
        let topology = topology_diagnostics(&diagnostics);

        assert!(
            topology.len() >= 6,
            "expected topology validation diagnostics: {diagnostics:#?}"
        );
        assert!(topology.iter().all(|diagnostic| {
            let labels = evidence_labels(diagnostic);
            labels.contains("family")
                && labels.contains("stable_key")
                && labels.contains("field")
                && labels.contains("reason")
        }));
    }

    #[test]
    fn topology_stable_key_conflicts_reject_nonidentical_duplicate_rows() {
        let mut db = AnalysisDb::new();
        db.replace_topology_facts(TopologyOutput {
            workspace_roots: vec![root(".", "root:repo")],
            packages: vec![
                package(
                    "package:dup",
                    Some(WorkspaceRootId(0)),
                    None,
                    "src/a",
                    TopologyPrecision::ExactStatic,
                    TopologyStatus::Present,
                ),
                package(
                    "package:dup",
                    Some(WorkspaceRootId(0)),
                    None,
                    "src/b",
                    TopologyPrecision::ExactStatic,
                    TopologyStatus::Present,
                ),
            ],
            ..TopologyOutput::default()
        });

        let diagnostics = validate_fact_metadata(&db, AnalysisKernel::provider_manifests());

        assert!(topology_diagnostics(&diagnostics).iter().any(|diagnostic| {
            diagnostic.evidence.iter().any(|evidence| {
                evidence.label == "reason" && evidence.value.contains("stable_key_conflict")
            })
        }));
    }

    #[test]
    fn topology_validation_rejects_unsupported_or_dynamic_rows_claiming_exactness() {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/app.ts"),
            "src/app.ts".to_string(),
            "import React from 'react';\n".to_string(),
        );
        let import = db.push_import(ImportFact::new(
            ImportId::from_raw(99),
            file,
            None,
            "react".to_string(),
            span(file),
            Language::TypeScript,
        ));
        db.replace_semantic_index_facts(
            Vec::new(),
            vec![SemanticImportFact {
                id: SemanticImportId(0),
                language: Language::TypeScript,
                file: Some(file),
                package: None,
                module: None,
                scope: None,
                import_path: "react".to_string(),
                local_name: None,
                imported_name: None,
                namespace: crate::core::SymbolNamespace::Value,
                kind: SemanticImportKind::DynamicImport,
                stable_key: db.stable_key_interner().intern("semantic:react"),
                status: SemanticStatus::Dynamic,
            }],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        db.replace_topology_facts(TopologyOutput {
            dependency_requirements: vec![DependencyRequirementFact {
                id: DependencyRequirementId(0),
                from_package: None,
                target_package: None,
                target_name: "react".to_string(),
                version_requirement: Some("^18".to_string()),
                kind: RequirementKind::Runtime,
                manifest_path: Some("package.json".to_string()),
                stable_key: crate::core::stable_key_for_test("requirement:react"),
                producer_id: "test",
                precision: TopologyPrecision::ExactLockfile,
                status: TopologyStatus::Unsupported,
            }],
            resolved_dependency_edges: vec![ResolvedDependencyEdgeFact {
                id: ResolvedDependencyEdgeId(0),
                requirement: Some(DependencyRequirementId(0)),
                from_package: None,
                to_package: None,
                package_name: "react".to_string(),
                resolved_version: None,
                kind: ResolvedDependencyKind::Unknown,
                stable_key: crate::core::stable_key_for_test("resolved:react"),
                producer_id: "test",
                precision: TopologyPrecision::ExactLockfile,
                status: TopologyStatus::Unsupported,
            }],
            import_to_package_edges: vec![import_edge(
                "import-to-package:dynamic",
                Some(import),
                None,
                Some("semantic:react".to_string()),
                Some(file),
                ImportToPackageStatus::Dynamic,
                TopologyPrecision::ExactStatic,
            )],
            ..TopologyOutput::default()
        });

        let diagnostics = validate_fact_metadata(&db, AnalysisKernel::provider_manifests());
        let reasons = topology_diagnostics(&diagnostics)
            .iter()
            .flat_map(|diagnostic| diagnostic.evidence.iter())
            .filter(|evidence| evidence.label == "reason")
            .map(|evidence| evidence.value.as_str())
            .collect::<Vec<_>>();

        assert!(
            reasons
                .iter()
                .any(|reason| reason.contains("unsupported_or_dynamic_exactness")),
            "expected exactness rejection: {diagnostics:#?}"
        );
    }

    #[test]
    fn topology_validation_accepts_exact_lockfile_for_lockfile_dependency_rows() {
        let mut db = AnalysisDb::new();
        db.replace_topology_facts(TopologyOutput {
            resolved_dependency_edges: [
                ResolvedDependencyKind::Lockfile,
                ResolvedDependencyKind::LockfileSelected,
                ResolvedDependencyKind::ChecksumEvidence,
            ]
            .into_iter()
            .enumerate()
            .map(|(index, kind)| ResolvedDependencyEdgeFact {
                id: ResolvedDependencyEdgeId(index as u64),
                requirement: None,
                from_package: None,
                to_package: None,
                package_name: format!("package-{index}"),
                resolved_version: Some("1.0.0".to_string()),
                kind,
                stable_key: crate::core::stable_key_for_test(&format!("resolved:package-{index}")),
                producer_id: "test",
                precision: TopologyPrecision::ExactLockfile,
                status: TopologyStatus::Resolved,
            })
            .collect(),
            ..TopologyOutput::default()
        });

        let diagnostics = validate_fact_metadata(&db, AnalysisKernel::provider_manifests());

        assert!(
            topology_diagnostics(&diagnostics).is_empty(),
            "expected exact lockfile rows to validate cleanly: {diagnostics:#?}"
        );
    }

    fn root(path: &str, stable_key: &str) -> WorkspaceRootFact {
        WorkspaceRootFact {
            id: WorkspaceRootId(0),
            kind: WorkspaceRootKind::Repository,
            root_path: path.to_string(),
            manifest_path: None,
            language: None,
            stable_key: crate::core::stable_key_for_test(stable_key),
            producer_id: "test",
            precision: TopologyPrecision::ExactStatic,
            status: TopologyStatus::Present,
        }
    }

    fn package(
        stable_key: &str,
        workspace_root: Option<WorkspaceRootId>,
        module_node: Option<ModuleNodeId>,
        path: &str,
        precision: TopologyPrecision,
        status: TopologyStatus,
    ) -> TopologyPackageFact {
        TopologyPackageFact {
            id: TopologyPackageId(0),
            workspace_root,
            package: None,
            module_node,
            kind: TopologyPackageKind::Workspace,
            name: stable_key.to_string(),
            version: None,
            path: path.to_string(),
            language: Some(Language::TypeScript),
            stable_key: crate::core::stable_key_for_test(stable_key),
            producer_id: "test",
            precision,
            status,
        }
    }

    fn import_edge(
        stable_key: &str,
        syntax_import: Option<ImportId>,
        resolved_import: Option<ResolvedImportId>,
        semantic_import_stable_key: Option<String>,
        from_file: Option<FileId>,
        status: ImportToPackageStatus,
        precision: TopologyPrecision,
    ) -> ImportToPackageFact {
        ImportToPackageFact {
            id: ImportToPackageId(0),
            syntax_import,
            resolved_import,
            semantic_import_stable_key: semantic_import_stable_key
                .as_deref()
                .map(crate::core::stable_key_for_test),
            from_file,
            from_package: Some(TopologyPackageId(404)),
            to_package: None,
            target_node: Some(ModuleNodeId::from_raw(404)),
            from_package_stable_key: None,
            to_package_stable_key: None,
            source_set_stable_key: None,
            import_path: "react".to_string(),
            context: ImportContextKind::Source,
            stable_key: crate::core::stable_key_for_test(stable_key),
            producer_id: "test",
            precision,
            status,
        }
    }

    fn span(file: FileId) -> Span {
        Span::new(file, 0, 1, 1, 1, 1, 2)
    }

    fn topology_diagnostics(
        diagnostics: &[crate::diagnostics::Diagnostic],
    ) -> Vec<&crate::diagnostics::Diagnostic> {
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.message.starts_with("Topology validation failed"))
            .collect()
    }

    fn evidence_labels(diagnostic: &crate::diagnostics::Diagnostic) -> BTreeSet<&str> {
        diagnostic
            .evidence
            .iter()
            .map(|evidence| evidence.label.as_str())
            .collect()
    }
}

#[derive(Debug, Default)]
pub(crate) struct IdSets {
    files: BTreeSet<FileId>,
    packages: BTreeSet<PackageId>,
    functions: BTreeSet<FunctionId>,
    branches: BTreeSet<BranchId>,
    imports: BTreeSet<ImportId>,
    bodies: BTreeSet<MirBodyId>,
    operations: BTreeSet<MirOpId>,
    places: BTreeSet<PlaceId>,
    cfg_nodes: BTreeSet<crate::analysis::cfg::ids::CfgNodeId>,
    call_sites: BTreeSet<crate::analysis::ids::CallSiteId>,
    resolved_imports: BTreeSet<ResolvedImportId>,
    module_nodes: BTreeSet<ModuleNodeId>,
    workspace_roots: BTreeSet<WorkspaceRootId>,
    topology_packages: BTreeSet<TopologyPackageId>,
    source_sets: BTreeSet<SourceSetId>,
    dependency_requirements: BTreeSet<DependencyRequirementId>,
    symbols: BTreeSet<SymbolId>,
    references: BTreeSet<ReferenceId>,
    scopes: BTreeSet<ScopeId>,
    exports: BTreeSet<ExportId>,
    semantic_import_stable_keys: BTreeSet<String>,
    symbol_stable_keys: BTreeSet<String>,
    reference_stable_keys: BTreeSet<String>,
}

impl IdSets {
    fn from_db(db: &AnalysisDb) -> Self {
        Self {
            files: db.files().iter().map(|fact| fact.id).collect(),
            packages: db.packages().iter().map(|fact| fact.id).collect(),
            functions: db.functions().iter().map(|fact| fact.id).collect(),
            branches: db.branches().iter().map(|fact| fact.id).collect(),
            imports: db.imports().iter().map(|fact| fact.id).collect(),
            bodies: db.mir_bodies().iter().map(|fact| fact.id).collect(),
            operations: db.mir_operations().iter().map(|fact| fact.id).collect(),
            places: db.mir_places().iter().map(|fact| fact.id).collect(),
            cfg_nodes: db.cfg_nodes().iter().map(|fact| fact.id).collect(),
            call_sites: db.call_sites().iter().map(|fact| fact.id).collect(),
            resolved_imports: db.resolved_imports().iter().map(|fact| fact.id).collect(),
            module_nodes: db.module_nodes().iter().map(|fact| fact.id).collect(),
            workspace_roots: db.workspace_roots().iter().map(|fact| fact.id).collect(),
            topology_packages: db.topology_packages().iter().map(|fact| fact.id).collect(),
            source_sets: db.source_sets().iter().map(|fact| fact.id).collect(),
            dependency_requirements: db
                .dependency_requirements()
                .iter()
                .map(|fact| fact.id)
                .collect(),
            symbols: db.symbols().iter().map(|fact| fact.id).collect(),
            references: db.references().iter().map(|fact| fact.id).collect(),
            scopes: db.scopes().iter().map(|fact| fact.id).collect(),
            exports: db.exports().iter().map(|fact| fact.id).collect(),
            semantic_import_stable_keys: db
                .semantic_imports()
                .iter()
                .map(|fact| db.resolve_stable_key(fact.stable_key).to_string())
                .collect(),
            symbol_stable_keys: db
                .symbols()
                .iter()
                .map(|fact| db.resolve_stable_key(fact.stable_key).to_string())
                .collect(),
            reference_stable_keys: db
                .references()
                .iter()
                .map(|fact| db.resolve_stable_key(fact.stable_key).to_string())
                .collect(),
        }
    }
}

fn validate_missing_metadata(db: &AnalysisDb, diagnostics: &mut Vec<PendingIssue>) {
    for missing in db.missing_fact_metadata() {
        let reference = FactRef::new(missing.family, missing.run_id);
        diagnostics.push((
            internal_diagnostic(format!(
                "Fact metadata missing for {}#{}.",
                missing.family.label(),
                missing.run_id
            ))
            .with_evidence("family", missing.family.label())
            .with_evidence("fact_ref", fact_ref_value(reference)),
            Attribution::Family(missing.family),
        ));
    }
}

fn validate_stable_key_conflicts(db: &AnalysisDb, diagnostics: &mut Vec<PendingIssue>) {
    let mut conflicts = db.fact_meta().stable_key_conflicts().collect::<Vec<_>>();
    conflicts.sort_by(|left, right| {
        (
            left.family,
            db.resolve_stable_key(left.stable_key),
            left.existing,
            left.incoming,
        )
            .cmp(&(
                right.family,
                db.resolve_stable_key(right.stable_key),
                right.existing,
                right.incoming,
            ))
    });
    for conflict in conflicts {
        diagnostics.push((
            internal_diagnostic(format!(
                "Fact metadata stable key conflict detected for {} stable key.",
                conflict.family.label()
            ))
            .with_evidence("family", conflict.family.label())
            .with_evidence(
                "stable_key",
                db.resolve_stable_key(conflict.stable_key).to_string(),
            )
            .with_evidence("existing_ref", fact_ref_value(conflict.existing))
            .with_evidence("incoming_ref", fact_ref_value(conflict.incoming)),
            Attribution::FamilyIdentity(conflict.family),
        ));
    }
}
fn validate_metadata_providers(
    db: &AnalysisDb,
    manifests_by_id: &BTreeMap<&'static str, ProviderManifest>,
    diagnostics: &mut Vec<PendingIssue>,
) {
    for (reference, metadata) in db.fact_meta().rows() {
        if !manifests_by_id.contains_key(metadata.producer_id)
            && !is_known_extension_producer(db, metadata.producer_id)
        {
            diagnostics.push((
                provider_manifest_diagnostic(reference, "producer_id", metadata.producer_id),
                Attribution::Family(reference.family),
            ));
        }
        if !manifests_by_id.contains_key(metadata.layer_id)
            && !is_known_extension_producer(db, metadata.layer_id)
        {
            diagnostics.push((
                provider_manifest_diagnostic(reference, "layer_id", metadata.layer_id),
                Attribution::Family(reference.family),
            ));
        }
    }
}

fn validate_references(db: &AnalysisDb, ids: &IdSets, diagnostics: &mut Vec<PendingIssue>) {
    for fact in db.functions() {
        check_ref(
            diagnostics,
            &ids.files,
            FactFamily::Function,
            fact.id.0,
            "FunctionFact.file",
            fact.file,
        );
    }
    for fact in db.packages() {
        check_ref(
            diagnostics,
            &ids.files,
            FactFamily::Package,
            fact.id.0,
            "PackageFact.file",
            fact.file,
        );
    }
    for fact in db.imports() {
        check_ref(
            diagnostics,
            &ids.files,
            FactFamily::Import,
            fact.id.0,
            "ImportFact.file",
            fact.file,
        );
    }
    for fact in db.branches() {
        check_ref(
            diagnostics,
            &ids.files,
            FactFamily::BranchObligation,
            fact.id.0,
            "BranchObligation.file",
            fact.file,
        );
        check_optional_ref(
            diagnostics,
            &ids.functions,
            FactFamily::BranchObligation,
            fact.id.0,
            "BranchObligation.function",
            fact.function,
        );
    }
    for (run_id, fact) in db.tests().iter().enumerate() {
        check_ref(
            diagnostics,
            &ids.files,
            FactFamily::Test,
            run_id as u64,
            "TestFact.file",
            fact.file,
        );
        check_optional_ref(
            diagnostics,
            &ids.functions,
            FactFamily::Test,
            run_id as u64,
            "TestFact.function",
            fact.function,
        );
    }
    for (run_id, fact) in db.coverage().iter().enumerate() {
        check_ref(
            diagnostics,
            &ids.branches,
            FactFamily::Coverage,
            run_id as u64,
            "CoverageFact.branch",
            fact.branch,
        );
    }
    for fact in db.evidence_nodes() {
        check_optional_ref(
            diagnostics,
            &ids.files,
            FactFamily::EvidenceNode,
            fact.id.0,
            "EvidenceNodeFact.file",
            fact.file,
        );
        check_optional_ref(
            diagnostics,
            &ids.functions,
            FactFamily::EvidenceNode,
            fact.id.0,
            "EvidenceNodeFact.function",
            fact.function,
        );
        check_optional_ref(
            diagnostics,
            &ids.bodies,
            FactFamily::EvidenceNode,
            fact.id.0,
            "EvidenceNodeFact.body",
            fact.body,
        );
        check_optional_ref(
            diagnostics,
            &ids.operations,
            FactFamily::EvidenceNode,
            fact.id.0,
            "EvidenceNodeFact.operation",
            fact.operation,
        );
        check_optional_ref(
            diagnostics,
            &ids.cfg_nodes,
            FactFamily::EvidenceNode,
            fact.id.0,
            "EvidenceNodeFact.cfg_node",
            fact.cfg_node,
        );
        check_optional_ref(
            diagnostics,
            &ids.places,
            FactFamily::EvidenceNode,
            fact.id.0,
            "EvidenceNodeFact.place",
            fact.place,
        );
        check_optional_ref(
            diagnostics,
            &ids.symbols,
            FactFamily::EvidenceNode,
            fact.id.0,
            "EvidenceNodeFact.symbol",
            fact.symbol,
        );
        check_optional_ref(
            diagnostics,
            &ids.references,
            FactFamily::EvidenceNode,
            fact.id.0,
            "EvidenceNodeFact.reference",
            fact.reference,
        );
        check_optional_ref(
            diagnostics,
            &ids.call_sites,
            FactFamily::EvidenceNode,
            fact.id.0,
            "EvidenceNodeFact.call_site",
            fact.call_site,
        );
    }
    for (run_id, fact) in db.ts_components().iter().enumerate() {
        check_ref(
            diagnostics,
            &ids.files,
            FactFamily::TsComponent,
            run_id as u64,
            "TsComponentFact.file",
            fact.file,
        );
        check_optional_ref(
            diagnostics,
            &ids.functions,
            FactFamily::TsComponent,
            run_id as u64,
            "TsComponentFact.function",
            fact.function,
        );
    }
    for (run_id, fact) in db.ts_classes().iter().enumerate() {
        check_ref(
            diagnostics,
            &ids.files,
            FactFamily::TsClass,
            run_id as u64,
            "TsClassFact.file",
            fact.file,
        );
    }
    for (run_id, fact) in db.string_literals().iter().enumerate() {
        check_ref(
            diagnostics,
            &ids.files,
            FactFamily::StringLiteral,
            run_id as u64,
            "StringLiteralFact.file",
            fact.file,
        );
    }
    for (run_id, fact) in db.jsx_attributes().iter().enumerate() {
        check_ref(
            diagnostics,
            &ids.files,
            FactFamily::JsxAttribute,
            run_id as u64,
            "JsxAttributeFact.file",
            fact.file,
        );
    }
    for (run_id, fact) in db.file_metrics().iter().enumerate() {
        check_ref(
            diagnostics,
            &ids.files,
            FactFamily::FileMetric,
            run_id as u64,
            "FileMetricFact.file",
            fact.file,
        );
    }
    for (run_id, fact) in db.function_metrics().iter().enumerate() {
        check_ref(
            diagnostics,
            &ids.files,
            FactFamily::FunctionMetric,
            run_id as u64,
            "FunctionMetricFact.file",
            fact.file,
        );
        check_ref(
            diagnostics,
            &ids.functions,
            FactFamily::FunctionMetric,
            run_id as u64,
            "FunctionMetricFact.function",
            fact.function,
        );
    }
    for (run_id, fact) in db.complexity_metrics().iter().enumerate() {
        check_ref(
            diagnostics,
            &ids.files,
            FactFamily::ComplexityMetric,
            run_id as u64,
            "ComplexityMetricFact.file",
            fact.file,
        );
        check_ref(
            diagnostics,
            &ids.functions,
            FactFamily::ComplexityMetric,
            run_id as u64,
            "ComplexityMetricFact.function",
            fact.function,
        );
    }
    for fact in db.resolved_imports() {
        check_ref(
            diagnostics,
            &ids.imports,
            FactFamily::ResolvedImport,
            fact.id.0,
            "ResolvedImportFact.import",
            fact.import,
        );
        check_ref(
            diagnostics,
            &ids.files,
            FactFamily::ResolvedImport,
            fact.id.0,
            "ResolvedImportFact.from_file",
            fact.from_file,
        );
        check_optional_ref(
            diagnostics,
            &ids.module_nodes,
            FactFamily::ResolvedImport,
            fact.id.0,
            "ResolvedImportFact.target_node",
            fact.target_node,
        );
    }
    for node in db.module_nodes() {
        check_optional_ref(
            diagnostics,
            &ids.files,
            FactFamily::ModuleNode,
            node.id.0,
            "ModuleNode.file",
            node.file,
        );
        check_optional_ref(
            diagnostics,
            &ids.packages,
            FactFamily::ModuleNode,
            node.id.0,
            "ModuleNode.package",
            node.package,
        );
    }
    for edge in db.module_edges() {
        check_ref(
            diagnostics,
            &ids.module_nodes,
            FactFamily::ModuleEdge,
            edge.id.0,
            "ModuleEdge.from",
            edge.from,
        );
        check_ref(
            diagnostics,
            &ids.module_nodes,
            FactFamily::ModuleEdge,
            edge.id.0,
            "ModuleEdge.to",
            edge.to,
        );
        check_optional_ref(
            diagnostics,
            &ids.imports,
            FactFamily::ModuleEdge,
            edge.id.0,
            "ModuleEdge.import",
            edge.import,
        );
        check_optional_ref(
            diagnostics,
            &ids.resolved_imports,
            FactFamily::ModuleEdge,
            edge.id.0,
            "ModuleEdge.resolved_import",
            edge.resolved_import,
        );
    }
    for symbol in db.symbols() {
        check_optional_ref(
            diagnostics,
            &ids.files,
            FactFamily::Symbol,
            symbol.id.0,
            "SymbolFact.file",
            symbol.file,
        );
        check_optional_ref(
            diagnostics,
            &ids.packages,
            FactFamily::Symbol,
            symbol.id.0,
            "SymbolFact.package",
            symbol.package,
        );
        check_optional_ref(
            diagnostics,
            &ids.module_nodes,
            FactFamily::Symbol,
            symbol.id.0,
            "SymbolFact.module",
            symbol.module,
        );
        check_optional_ref(
            diagnostics,
            &ids.symbols,
            FactFamily::Symbol,
            symbol.id.0,
            "SymbolFact.owner",
            symbol.owner,
        );
    }
    for definition in db.definitions() {
        check_ref(
            diagnostics,
            &ids.symbols,
            FactFamily::Definition,
            definition.id.0,
            "DefinitionFact.symbol",
            definition.symbol,
        );
        check_optional_ref(
            diagnostics,
            &ids.files,
            FactFamily::Definition,
            definition.id.0,
            "DefinitionFact.file",
            definition.file,
        );
        check_optional_ref(
            diagnostics,
            &ids.packages,
            FactFamily::Definition,
            definition.id.0,
            "DefinitionFact.package",
            definition.package,
        );
        check_optional_ref(
            diagnostics,
            &ids.module_nodes,
            FactFamily::Definition,
            definition.id.0,
            "DefinitionFact.module",
            definition.module,
        );
        check_optional_ref(
            diagnostics,
            &ids.symbols,
            FactFamily::Definition,
            definition.id.0,
            "DefinitionFact.owner",
            definition.owner,
        );
    }
    for reference in db.references() {
        check_optional_ref(
            diagnostics,
            &ids.files,
            FactFamily::Reference,
            reference.id.0,
            "ReferenceFact.file",
            reference.file,
        );
        check_optional_ref(
            diagnostics,
            &ids.packages,
            FactFamily::Reference,
            reference.id.0,
            "ReferenceFact.package",
            reference.package,
        );
        check_optional_ref(
            diagnostics,
            &ids.module_nodes,
            FactFamily::Reference,
            reference.id.0,
            "ReferenceFact.module",
            reference.module,
        );
        check_optional_ref(
            diagnostics,
            &ids.symbols,
            FactFamily::Reference,
            reference.id.0,
            "ReferenceFact.owner",
            reference.owner,
        );
        check_optional_ref(
            diagnostics,
            &ids.symbols,
            FactFamily::Reference,
            reference.id.0,
            "ReferenceFact.target",
            reference.target,
        );
        for candidate in &reference.candidates {
            check_ref(
                diagnostics,
                &ids.symbols,
                FactFamily::Reference,
                reference.id.0,
                "ReferenceFact.candidates",
                *candidate,
            );
        }
    }
}

fn validate_spans(
    db: &AnalysisDb,
    file_ids: &BTreeSet<FileId>,
    diagnostics: &mut Vec<PendingIssue>,
) {
    for fact in db.functions() {
        check_span(
            db,
            file_ids,
            diagnostics,
            SpanCheck {
                family: FactFamily::Function,
                run_id: fact.id.0,
                field: "FunctionFact.span",
                owner_file: Some(fact.file),
                span: &fact.span,
            },
        );
    }
    for fact in db.packages() {
        check_span(
            db,
            file_ids,
            diagnostics,
            SpanCheck {
                family: FactFamily::Package,
                run_id: fact.id.0,
                field: "PackageFact.span",
                owner_file: Some(fact.file),
                span: &fact.span,
            },
        );
    }
    for fact in db.imports() {
        check_span(
            db,
            file_ids,
            diagnostics,
            SpanCheck {
                family: FactFamily::Import,
                run_id: fact.id.0,
                field: "ImportFact.span",
                owner_file: Some(fact.file),
                span: &fact.span,
            },
        );
    }
    for fact in db.branches() {
        check_span(
            db,
            file_ids,
            diagnostics,
            SpanCheck {
                family: FactFamily::BranchObligation,
                run_id: fact.id.0,
                field: "BranchObligation.decision_span",
                owner_file: Some(fact.file),
                span: &fact.decision_span,
            },
        );
    }
    for (run_id, fact) in db.tests().iter().enumerate() {
        check_span(
            db,
            file_ids,
            diagnostics,
            SpanCheck {
                family: FactFamily::Test,
                run_id: run_id as u64,
                field: "TestFact.span",
                owner_file: Some(fact.file),
                span: &fact.span,
            },
        );
    }
    for (run_id, fact) in db.ts_components().iter().enumerate() {
        check_span(
            db,
            file_ids,
            diagnostics,
            SpanCheck {
                family: FactFamily::TsComponent,
                run_id: run_id as u64,
                field: "TsComponentFact.span",
                owner_file: Some(fact.file),
                span: &fact.span,
            },
        );
    }
    for (run_id, fact) in db.ts_classes().iter().enumerate() {
        check_span(
            db,
            file_ids,
            diagnostics,
            SpanCheck {
                family: FactFamily::TsClass,
                run_id: run_id as u64,
                field: "TsClassFact.span",
                owner_file: Some(fact.file),
                span: &fact.span,
            },
        );
    }
    for (run_id, fact) in db.string_literals().iter().enumerate() {
        check_span(
            db,
            file_ids,
            diagnostics,
            SpanCheck {
                family: FactFamily::StringLiteral,
                run_id: run_id as u64,
                field: "StringLiteralFact.span",
                owner_file: Some(fact.file),
                span: &fact.span,
            },
        );
    }
    for (run_id, fact) in db.jsx_attributes().iter().enumerate() {
        check_span(
            db,
            file_ids,
            diagnostics,
            SpanCheck {
                family: FactFamily::JsxAttribute,
                run_id: run_id as u64,
                field: "JsxAttributeFact.span",
                owner_file: Some(fact.file),
                span: &fact.span,
            },
        );
    }
    for (run_id, fact) in db.function_metrics().iter().enumerate() {
        check_span(
            db,
            file_ids,
            diagnostics,
            SpanCheck {
                family: FactFamily::FunctionMetric,
                run_id: run_id as u64,
                field: "FunctionMetricFact.span",
                owner_file: Some(fact.file),
                span: &fact.span,
            },
        );
    }
    for (run_id, fact) in db.complexity_metrics().iter().enumerate() {
        check_span(
            db,
            file_ids,
            diagnostics,
            SpanCheck {
                family: FactFamily::ComplexityMetric,
                run_id: run_id as u64,
                field: "ComplexityMetricFact.span",
                owner_file: Some(fact.file),
                span: &fact.span,
            },
        );
    }
    for fact in db.evidence_nodes() {
        if let Some(span) = &fact.span {
            check_span(
                db,
                file_ids,
                diagnostics,
                SpanCheck {
                    family: FactFamily::EvidenceNode,
                    run_id: fact.id.0,
                    field: "EvidenceNodeFact.span",
                    owner_file: fact.file,
                    span,
                },
            );
        }
    }
    for symbol in db.symbols() {
        if let Some(span) = &symbol.primary_span {
            check_span(
                db,
                file_ids,
                diagnostics,
                SpanCheck {
                    family: FactFamily::Symbol,
                    run_id: symbol.id.0,
                    field: "SymbolFact.primary_span",
                    owner_file: symbol.file,
                    span,
                },
            );
        }
    }
    for definition in db.definitions() {
        if let Some(span) = &definition.primary_span {
            check_span(
                db,
                file_ids,
                diagnostics,
                SpanCheck {
                    family: FactFamily::Definition,
                    run_id: definition.id.0,
                    field: "DefinitionFact.primary_span",
                    owner_file: definition.file,
                    span,
                },
            );
        }
    }
    for reference in db.references() {
        if let Some(span) = &reference.primary_span {
            check_span(
                db,
                file_ids,
                diagnostics,
                SpanCheck {
                    family: FactFamily::Reference,
                    run_id: reference.id.0,
                    field: "ReferenceFact.primary_span",
                    owner_file: reference.file,
                    span,
                },
            );
        }
    }
}

fn validate_precision_ceilings(
    db: &AnalysisDb,
    manifests_by_id: &BTreeMap<&'static str, ProviderManifest>,
    diagnostics: &mut Vec<PendingIssue>,
) {
    for (reference, metadata) in db.fact_meta().rows() {
        if metadata.producer_id.starts_with("polint.extension.") {
            validate_extension_precision(reference, metadata, db, diagnostics);
            continue;
        }
        let Some(manifest) = manifests_by_id.get(metadata.producer_id) else {
            continue;
        };
        if precision_within_ceiling(metadata.precision, manifest.precision_ceiling) {
            continue;
        }
        diagnostics.push((
            internal_diagnostic(format!(
                "Fact metadata precision ceiling violated for {}#{}.",
                reference.family.label(),
                reference.run_id
            ))
            .with_evidence("producer_id", metadata.producer_id)
            .with_evidence("family", reference.family.label())
            .with_evidence("precision", precision_label(metadata.precision))
            .with_evidence("ceiling", ceiling_label(manifest.precision_ceiling)),
            Attribution::Fact(reference),
        ));
    }
}

fn is_known_extension_producer(db: &AnalysisDb, producer_id: &str) -> bool {
    producer_id.starts_with("polint.extension.")
        && db.extension_facts().iter().any(|fact| {
            producer_id
                == format!(
                    "polint.extension.{}.{}",
                    fact.extension_id, fact.provider_id
                )
        })
}

fn validate_extension_precision(
    reference: FactRef,
    metadata: &crate::analysis_kernel::FactMeta,
    db: &AnalysisDb,
    diagnostics: &mut Vec<PendingIssue>,
) {
    let Some(fact) = db.extension_facts().get(reference.run_id as usize) else {
        return;
    };
    if metadata.precision != FactPrecision::Exact {
        return;
    }
    if fact
        .evidence
        .iter()
        .any(|evidence| evidence == "exact_validation")
    {
        return;
    }
    diagnostics.push((
        internal_diagnostic(format!(
            "Fact metadata precision ceiling violated for {}#{}.",
            reference.family.label(),
            reference.run_id
        ))
        .with_evidence("producer_id", metadata.producer_id)
        .with_evidence("family", reference.family.label())
        .with_evidence("precision", precision_label(metadata.precision))
        .with_evidence("ceiling", "extension_exact_requires_validation_evidence"),
        Attribution::Fact(reference),
    ));
}

fn validate_semantic_index(db: &AnalysisDb, ids: &IdSets, diagnostics: &mut Vec<Diagnostic>) {
    let semantic_keys = semantic_reference_keys(db, ids);

    for scope in db.scopes() {
        check_semantic_optional_ref(
            diagnostics,
            &ids.files,
            FactFamily::Scope,
            db.resolve_stable_key(scope.stable_key).as_ref(),
            "ScopeFact.file",
            scope.file,
        );
        check_semantic_optional_ref(
            diagnostics,
            &ids.packages,
            FactFamily::Scope,
            db.resolve_stable_key(scope.stable_key).as_ref(),
            "ScopeFact.package",
            scope.package,
        );
        check_semantic_optional_ref(
            diagnostics,
            &ids.module_nodes,
            FactFamily::Scope,
            db.resolve_stable_key(scope.stable_key).as_ref(),
            "ScopeFact.module",
            scope.module,
        );
        check_semantic_optional_ref(
            diagnostics,
            &ids.scopes,
            FactFamily::Scope,
            db.resolve_stable_key(scope.stable_key).as_ref(),
            "ScopeFact.parent",
            scope.parent,
        );
    }

    for import in db.semantic_imports() {
        check_semantic_optional_ref(
            diagnostics,
            &ids.files,
            FactFamily::SemanticImport,
            db.resolve_stable_key(import.stable_key).as_ref(),
            "SemanticImportFact.file",
            import.file,
        );
        check_semantic_optional_ref(
            diagnostics,
            &ids.packages,
            FactFamily::SemanticImport,
            db.resolve_stable_key(import.stable_key).as_ref(),
            "SemanticImportFact.package",
            import.package,
        );
        check_semantic_optional_ref(
            diagnostics,
            &ids.module_nodes,
            FactFamily::SemanticImport,
            db.resolve_stable_key(import.stable_key).as_ref(),
            "SemanticImportFact.module",
            import.module,
        );
        check_semantic_optional_ref(
            diagnostics,
            &ids.scopes,
            FactFamily::SemanticImport,
            db.resolve_stable_key(import.stable_key).as_ref(),
            "SemanticImportFact.scope",
            import.scope,
        );
    }

    for export in db.exports() {
        check_semantic_optional_ref(
            diagnostics,
            &ids.files,
            FactFamily::Export,
            db.resolve_stable_key(export.stable_key).as_ref(),
            "ExportFact.file",
            export.file,
        );
        check_semantic_optional_ref(
            diagnostics,
            &ids.packages,
            FactFamily::Export,
            db.resolve_stable_key(export.stable_key).as_ref(),
            "ExportFact.package",
            export.package,
        );
        check_semantic_optional_ref(
            diagnostics,
            &ids.module_nodes,
            FactFamily::Export,
            db.resolve_stable_key(export.stable_key).as_ref(),
            "ExportFact.module",
            export.module,
        );
        check_semantic_optional_ref(
            diagnostics,
            &ids.scopes,
            FactFamily::Export,
            db.resolve_stable_key(export.stable_key).as_ref(),
            "ExportFact.scope",
            export.scope,
        );
        if !semantic_status_allows_missing_references(export.status) {
            check_semantic_optional_ref(
                diagnostics,
                &ids.symbols,
                FactFamily::Export,
                db.resolve_stable_key(export.stable_key).as_ref(),
                "ExportFact.symbol",
                export.symbol,
            );
        }
    }

    for alias in db.aliases() {
        check_semantic_optional_ref(
            diagnostics,
            &ids.files,
            FactFamily::Alias,
            db.resolve_stable_key(alias.stable_key).as_ref(),
            "AliasFact.file",
            alias.file,
        );
        check_semantic_optional_ref(
            diagnostics,
            &ids.packages,
            FactFamily::Alias,
            db.resolve_stable_key(alias.stable_key).as_ref(),
            "AliasFact.package",
            alias.package,
        );
        check_semantic_optional_ref(
            diagnostics,
            &ids.module_nodes,
            FactFamily::Alias,
            db.resolve_stable_key(alias.stable_key).as_ref(),
            "AliasFact.module",
            alias.module,
        );
        if !semantic_status_allows_missing_references(alias.status) {
            check_semantic_key_ref(
                diagnostics,
                &semantic_keys,
                FactFamily::Alias,
                db.resolve_stable_key(alias.stable_key).as_ref(),
                "AliasFact.source_symbol_stable_key",
                db.resolve_stable_key(alias.source_symbol_stable_key)
                    .as_ref(),
            );
            for target in &alias.target_symbol_stable_keys {
                check_semantic_key_ref(
                    diagnostics,
                    &semantic_keys,
                    FactFamily::Alias,
                    db.resolve_stable_key(alias.stable_key).as_ref(),
                    "AliasFact.target_symbol_stable_keys",
                    db.resolve_stable_key(*target).as_ref(),
                );
            }
        }
    }

    for resolution in db.resolution_facts() {
        check_semantic_optional_ref(
            diagnostics,
            &ids.files,
            FactFamily::Resolution,
            db.resolve_stable_key(resolution.stable_key).as_ref(),
            "ResolutionFact.file",
            resolution.file,
        );
        check_semantic_optional_ref(
            diagnostics,
            &ids.packages,
            FactFamily::Resolution,
            db.resolve_stable_key(resolution.stable_key).as_ref(),
            "ResolutionFact.package",
            resolution.package,
        );
        check_semantic_optional_ref(
            diagnostics,
            &ids.module_nodes,
            FactFamily::Resolution,
            db.resolve_stable_key(resolution.stable_key).as_ref(),
            "ResolutionFact.module",
            resolution.module,
        );
        if !semantic_status_allows_missing_references(resolution.status) {
            check_semantic_key_ref(
                diagnostics,
                &semantic_keys,
                FactFamily::Resolution,
                db.resolve_stable_key(resolution.stable_key).as_ref(),
                "ResolutionFact.source_stable_key",
                db.resolve_stable_key(resolution.source_stable_key).as_ref(),
            );
            for target in &resolution.target_stable_keys {
                check_semantic_key_ref(
                    diagnostics,
                    &semantic_keys,
                    FactFamily::Resolution,
                    db.resolve_stable_key(resolution.stable_key).as_ref(),
                    "ResolutionFact.target_stable_keys",
                    db.resolve_stable_key(*target).as_ref(),
                );
            }
        }
    }

    for generated in db.generated_symbols() {
        check_semantic_optional_ref(
            diagnostics,
            &ids.files,
            FactFamily::GeneratedSymbol,
            db.resolve_stable_key(generated.stable_key).as_ref(),
            "GeneratedSymbolFact.file",
            generated.file,
        );
        check_semantic_optional_ref(
            diagnostics,
            &ids.packages,
            FactFamily::GeneratedSymbol,
            db.resolve_stable_key(generated.stable_key).as_ref(),
            "GeneratedSymbolFact.package",
            generated.package,
        );
        check_semantic_optional_ref(
            diagnostics,
            &ids.module_nodes,
            FactFamily::GeneratedSymbol,
            db.resolve_stable_key(generated.stable_key).as_ref(),
            "GeneratedSymbolFact.module",
            generated.module,
        );
        check_semantic_key_ref(
            diagnostics,
            &semantic_keys,
            FactFamily::GeneratedSymbol,
            db.resolve_stable_key(generated.stable_key).as_ref(),
            "GeneratedSymbolFact.source_stable_key",
            db.resolve_stable_key(generated.source_stable_key).as_ref(),
        );
        if generated.producer_id != SYMBOL_GRAPH_PROVIDER_ID {
            diagnostics.push(semantic_diagnostic(
                FactFamily::GeneratedSymbol,
                db.resolve_stable_key(generated.stable_key).as_ref(),
                format!("GeneratedSymbolFact.producer_id must be {SYMBOL_GRAPH_PROVIDER_ID}"),
            ));
        }
        if generated.generated_discriminator.is_empty() {
            diagnostics.push(semantic_diagnostic(
                FactFamily::GeneratedSymbol,
                db.resolve_stable_key(generated.stable_key).as_ref(),
                "GeneratedSymbolFact.generated_discriminator is empty",
            ));
        }
        if generated.status != SemanticStatus::Generated {
            diagnostics.push(semantic_diagnostic(
                FactFamily::GeneratedSymbol,
                db.resolve_stable_key(generated.stable_key).as_ref(),
                "GeneratedSymbolFact.status must be Generated",
            ));
        }
        if let Some(span) = &generated.span
            && let Some(reason) = span_failure_reason(db, &ids.files, generated.file, span)
        {
            diagnostics.push(semantic_diagnostic(
                FactFamily::GeneratedSymbol,
                db.resolve_stable_key(generated.stable_key).as_ref(),
                format!("GeneratedSymbolFact.span {reason}"),
            ));
        }
    }

    for stable_export in db.stable_exports() {
        check_semantic_ref(
            diagnostics,
            &ids.exports,
            FactFamily::StableExport,
            db.resolve_stable_key(stable_export.stable_key).as_ref(),
            "StableExportIdentity.export",
            stable_export.export,
        );
        if stable_export.status != SemanticStatus::Generated
            && !ids.symbol_stable_keys.contains(
                db.resolve_stable_key(stable_export.symbol_stable_key)
                    .as_ref(),
            )
            && !semantic_keys.contains(
                db.resolve_stable_key(stable_export.symbol_stable_key)
                    .as_ref(),
            )
        {
            diagnostics.push(semantic_diagnostic(
                FactFamily::StableExport,
                db.resolve_stable_key(stable_export.stable_key).as_ref(),
                format!(
                    "StableExportIdentity.symbol_stable_key does not exist: {}",
                    db.resolve_stable_key(stable_export.symbol_stable_key)
                ),
            ));
        }
    }

    for (reference, metadata) in db.fact_meta().rows() {
        if is_semantic_fact_family(reference.family)
            && metadata.producer_id == SYMBOL_GRAPH_PROVIDER_ID
            && metadata.precision == FactPrecision::Exact
        {
            diagnostics.push(semantic_diagnostic(
                reference.family,
                db.resolve_stable_key(metadata.stable_key).as_ref(),
                "provider precision ceiling exceeded: semantic rows from polint.symbol_graph are setup-aware, not exact",
            ));
        }
    }
}

fn validate_topology_facts(db: &AnalysisDb, ids: &IdSets, diagnostics: &mut Vec<Diagnostic>) {
    check_topology_family_stable_keys(
        diagnostics,
        FactFamily::WorkspaceRoot,
        db.workspace_roots(),
        |row| db.resolve_stable_key(row.stable_key),
    );
    check_topology_family_stable_keys(
        diagnostics,
        FactFamily::TopologyPackage,
        db.topology_packages(),
        |row| db.resolve_stable_key(row.stable_key),
    );
    check_topology_family_stable_keys(
        diagnostics,
        FactFamily::SourceSet,
        db.source_sets(),
        |row| db.resolve_stable_key(row.stable_key),
    );
    check_topology_family_stable_keys(
        diagnostics,
        FactFamily::DependencyRequirement,
        db.dependency_requirements(),
        |row| db.resolve_stable_key(row.stable_key),
    );
    check_topology_family_stable_keys(
        diagnostics,
        FactFamily::ResolvedDependencyEdge,
        db.resolved_dependency_edges(),
        |row| db.resolve_stable_key(row.stable_key),
    );
    check_topology_family_stable_keys(
        diagnostics,
        FactFamily::ImportToPackage,
        db.import_to_package_edges(),
        |row| db.resolve_stable_key(row.stable_key),
    );
    check_topology_family_stable_keys(
        diagnostics,
        FactFamily::RepoTopologyOverlay,
        db.repo_topology_overlays(),
        |row| db.resolve_stable_key(row.stable_key),
    );

    for root in db.workspace_roots() {
        check_topology_path(
            diagnostics,
            FactFamily::WorkspaceRoot,
            db.resolve_stable_key(root.stable_key).as_ref(),
            "WorkspaceRootFact.root_path",
            root.root_path.as_str(),
        );
        check_topology_optional_path(
            diagnostics,
            FactFamily::WorkspaceRoot,
            db.resolve_stable_key(root.stable_key).as_ref(),
            "WorkspaceRootFact.manifest_path",
            root.manifest_path.as_deref(),
        );
        check_topology_precision(
            diagnostics,
            FactFamily::WorkspaceRoot,
            db.resolve_stable_key(root.stable_key).as_ref(),
            "WorkspaceRootFact.precision",
            root.precision,
            root.status,
            false,
        );
    }

    for package in db.topology_packages() {
        check_topology_optional_ref(
            diagnostics,
            &ids.workspace_roots,
            FactFamily::TopologyPackage,
            db.resolve_stable_key(package.stable_key).as_ref(),
            "TopologyPackageFact.workspace_root",
            package.workspace_root,
        );
        check_topology_optional_ref(
            diagnostics,
            &ids.packages,
            FactFamily::TopologyPackage,
            db.resolve_stable_key(package.stable_key).as_ref(),
            "TopologyPackageFact.package",
            package.package,
        );
        check_topology_optional_ref(
            diagnostics,
            &ids.module_nodes,
            FactFamily::TopologyPackage,
            db.resolve_stable_key(package.stable_key).as_ref(),
            "TopologyPackageFact.module_node",
            package.module_node,
        );
        check_topology_path(
            diagnostics,
            FactFamily::TopologyPackage,
            db.resolve_stable_key(package.stable_key).as_ref(),
            "TopologyPackageFact.path",
            package.path.as_str(),
        );
        check_topology_precision(
            diagnostics,
            FactFamily::TopologyPackage,
            db.resolve_stable_key(package.stable_key).as_ref(),
            "TopologyPackageFact.precision",
            package.precision,
            package.status,
            false,
        );
    }

    for source_set in db.source_sets() {
        check_topology_optional_ref(
            diagnostics,
            &ids.topology_packages,
            FactFamily::SourceSet,
            db.resolve_stable_key(source_set.stable_key).as_ref(),
            "SourceSetFact.package",
            source_set.package,
        );
        check_topology_optional_ref(
            diagnostics,
            &ids.workspace_roots,
            FactFamily::SourceSet,
            db.resolve_stable_key(source_set.stable_key).as_ref(),
            "SourceSetFact.root",
            source_set.root,
        );
        for file in &source_set.files {
            check_topology_ref(
                diagnostics,
                &ids.files,
                FactFamily::SourceSet,
                db.resolve_stable_key(source_set.stable_key).as_ref(),
                "SourceSetFact.files",
                *file,
            );
        }
        check_topology_path(
            diagnostics,
            FactFamily::SourceSet,
            db.resolve_stable_key(source_set.stable_key).as_ref(),
            "SourceSetFact.path",
            source_set.path.as_str(),
        );
        check_topology_precision(
            diagnostics,
            FactFamily::SourceSet,
            db.resolve_stable_key(source_set.stable_key).as_ref(),
            "SourceSetFact.precision",
            source_set.precision,
            source_set.status,
            false,
        );
    }

    for requirement in db.dependency_requirements() {
        check_topology_optional_ref(
            diagnostics,
            &ids.topology_packages,
            FactFamily::DependencyRequirement,
            db.resolve_stable_key(requirement.stable_key).as_ref(),
            "DependencyRequirementFact.from_package",
            requirement.from_package,
        );
        check_topology_optional_ref(
            diagnostics,
            &ids.topology_packages,
            FactFamily::DependencyRequirement,
            db.resolve_stable_key(requirement.stable_key).as_ref(),
            "DependencyRequirementFact.target_package",
            requirement.target_package,
        );
        check_topology_optional_path(
            diagnostics,
            FactFamily::DependencyRequirement,
            db.resolve_stable_key(requirement.stable_key).as_ref(),
            "DependencyRequirementFact.manifest_path",
            requirement.manifest_path.as_deref(),
        );
        check_topology_precision(
            diagnostics,
            FactFamily::DependencyRequirement,
            db.resolve_stable_key(requirement.stable_key).as_ref(),
            "DependencyRequirementFact.precision",
            requirement.precision,
            requirement.status,
            false,
        );
    }

    for edge in db.resolved_dependency_edges() {
        check_topology_optional_ref(
            diagnostics,
            &ids.dependency_requirements,
            FactFamily::ResolvedDependencyEdge,
            db.resolve_stable_key(edge.stable_key).as_ref(),
            "ResolvedDependencyEdgeFact.requirement",
            edge.requirement,
        );
        check_topology_optional_ref(
            diagnostics,
            &ids.topology_packages,
            FactFamily::ResolvedDependencyEdge,
            db.resolve_stable_key(edge.stable_key).as_ref(),
            "ResolvedDependencyEdgeFact.from_package",
            edge.from_package,
        );
        check_topology_optional_ref(
            diagnostics,
            &ids.topology_packages,
            FactFamily::ResolvedDependencyEdge,
            db.resolve_stable_key(edge.stable_key).as_ref(),
            "ResolvedDependencyEdgeFact.to_package",
            edge.to_package,
        );
        check_resolved_dependency_precision(db, diagnostics, edge);
    }

    for edge in db.import_to_package_edges() {
        check_topology_optional_ref(
            diagnostics,
            &ids.imports,
            FactFamily::ImportToPackage,
            db.resolve_stable_key(edge.stable_key).as_ref(),
            "ImportToPackageFact.syntax_import",
            edge.syntax_import,
        );
        check_topology_optional_ref(
            diagnostics,
            &ids.resolved_imports,
            FactFamily::ImportToPackage,
            db.resolve_stable_key(edge.stable_key).as_ref(),
            "ImportToPackageFact.resolved_import",
            edge.resolved_import,
        );
        check_topology_optional_ref(
            diagnostics,
            &ids.files,
            FactFamily::ImportToPackage,
            db.resolve_stable_key(edge.stable_key).as_ref(),
            "ImportToPackageFact.from_file",
            edge.from_file,
        );
        check_topology_optional_ref(
            diagnostics,
            &ids.topology_packages,
            FactFamily::ImportToPackage,
            db.resolve_stable_key(edge.stable_key).as_ref(),
            "ImportToPackageFact.from_package",
            edge.from_package,
        );
        check_topology_optional_ref(
            diagnostics,
            &ids.topology_packages,
            FactFamily::ImportToPackage,
            db.resolve_stable_key(edge.stable_key).as_ref(),
            "ImportToPackageFact.to_package",
            edge.to_package,
        );
        check_topology_optional_ref(
            diagnostics,
            &ids.module_nodes,
            FactFamily::ImportToPackage,
            db.resolve_stable_key(edge.stable_key).as_ref(),
            "ImportToPackageFact.target_node",
            edge.target_node,
        );
        if let Some(key) = edge.semantic_import_stable_key
            && !ids
                .semantic_import_stable_keys
                .contains(db.resolve_stable_key(key).as_ref())
        {
            diagnostics.push(topology_diagnostic(
                FactFamily::ImportToPackage,
                db.resolve_stable_key(edge.stable_key).as_ref(),
                "ImportToPackageFact.semantic_import_stable_key",
                "semantic_import_stable_key_missing",
            ));
        }
        if edge.status == ImportToPackageStatus::Resolved && edge.to_package_stable_key.is_none() {
            diagnostics.push(topology_diagnostic(
                FactFamily::ImportToPackage,
                db.resolve_stable_key(edge.stable_key).as_ref(),
                "ImportToPackageFact.to_package_stable_key",
                "resolved_row_missing_to_package_stable_key",
            ));
        }
        if edge.status == ImportToPackageStatus::Undeclared
            && declared_requirement_exists(edge, db.dependency_requirements())
        {
            diagnostics.push(topology_diagnostic(
                FactFamily::ImportToPackage,
                db.resolve_stable_key(edge.stable_key).as_ref(),
                "ImportToPackageFact.status",
                "undeclared_row_has_matching_dependency_requirement",
            ));
        }
        check_import_to_package_precision(db, diagnostics, edge);
    }

    for overlay in db.repo_topology_overlays() {
        check_topology_optional_ref(
            diagnostics,
            &ids.workspace_roots,
            FactFamily::RepoTopologyOverlay,
            db.resolve_stable_key(overlay.stable_key).as_ref(),
            "RepoTopologyOverlayFact.root",
            overlay.root,
        );
        check_topology_optional_ref(
            diagnostics,
            &ids.topology_packages,
            FactFamily::RepoTopologyOverlay,
            db.resolve_stable_key(overlay.stable_key).as_ref(),
            "RepoTopologyOverlayFact.package",
            overlay.package,
        );
        check_topology_optional_ref(
            diagnostics,
            &ids.source_sets,
            FactFamily::RepoTopologyOverlay,
            db.resolve_stable_key(overlay.stable_key).as_ref(),
            "RepoTopologyOverlayFact.source_set",
            overlay.source_set,
        );
        check_topology_optional_path(
            diagnostics,
            FactFamily::RepoTopologyOverlay,
            db.resolve_stable_key(overlay.stable_key).as_ref(),
            "RepoTopologyOverlayFact.path",
            overlay.path.as_deref(),
        );
        check_topology_precision(
            diagnostics,
            FactFamily::RepoTopologyOverlay,
            db.resolve_stable_key(overlay.stable_key).as_ref(),
            "RepoTopologyOverlayFact.precision",
            overlay.precision,
            overlay.status,
            false,
        );
    }
}

fn check_topology_family_stable_keys<T: Serialize>(
    diagnostics: &mut Vec<Diagnostic>,
    family: FactFamily,
    rows: &[T],
    stable_key: impl Fn(&T) -> std::sync::Arc<str>,
) {
    let mut seen = BTreeMap::<String, serde_json::Value>::new();
    for row in rows {
        let key = stable_key(row).to_string();
        let normalized = normalized_topology_row(row);
        if let Some(existing) = seen.get(&key) {
            if existing != &normalized {
                diagnostics.push(topology_diagnostic(
                    family,
                    key.as_str(),
                    "stable_key",
                    "stable_key_conflict",
                ));
            } else {
                diagnostics.push(topology_diagnostic(
                    family,
                    key.as_str(),
                    "stable_key",
                    "duplicate_stable_key",
                ));
            }
        } else {
            seen.insert(key, normalized);
        }
    }
}

fn normalized_topology_row<T: Serialize>(row: &T) -> serde_json::Value {
    let mut value = serde_json::to_value(row).unwrap_or(serde_json::Value::Null);
    if let serde_json::Value::Object(object) = &mut value {
        object.remove("id");
    }
    value
}

fn check_topology_ref<T>(
    diagnostics: &mut Vec<Diagnostic>,
    valid_ids: &BTreeSet<T>,
    family: FactFamily,
    stable_key: &str,
    field: &'static str,
    value: T,
) where
    T: Copy + Debug + Ord,
{
    if valid_ids.contains(&value) {
        return;
    }
    diagnostics.push(topology_diagnostic(
        family,
        stable_key,
        field,
        format!("reference_missing:{value:?}"),
    ));
}

fn check_topology_optional_ref<T>(
    diagnostics: &mut Vec<Diagnostic>,
    valid_ids: &BTreeSet<T>,
    family: FactFamily,
    stable_key: &str,
    field: &'static str,
    value: Option<T>,
) where
    T: Copy + Debug + Ord,
{
    let Some(value) = value else {
        return;
    };
    check_topology_ref(diagnostics, valid_ids, family, stable_key, field, value);
}

fn check_topology_path(
    diagnostics: &mut Vec<Diagnostic>,
    family: FactFamily,
    stable_key: &str,
    field: &'static str,
    path: &str,
) {
    if repo_relative_path_is_valid(path) {
        return;
    }
    diagnostics.push(topology_diagnostic(
        family,
        stable_key,
        field,
        "malformed_repo_relative_path",
    ));
}

fn check_topology_optional_path(
    diagnostics: &mut Vec<Diagnostic>,
    family: FactFamily,
    stable_key: &str,
    field: &'static str,
    path: Option<&str>,
) {
    let Some(path) = path else {
        return;
    };
    check_topology_path(diagnostics, family, stable_key, field, path);
}

fn repo_relative_path_is_valid(path: &str) -> bool {
    if path.is_empty() || path == "." {
        return true;
    }
    if path.contains('\\') || path.starts_with('/') || path.contains(':') {
        return false;
    }
    !path.split('/').any(|component| component == "..")
}

fn check_resolved_dependency_precision(
    db: &AnalysisDb,
    diagnostics: &mut Vec<Diagnostic>,
    edge: &crate::module_graph::topology::ResolvedDependencyEdgeFact,
) {
    let exact_lockfile_allowed = matches!(
        edge.kind,
        ResolvedDependencyKind::Lockfile
            | ResolvedDependencyKind::LockfileSelected
            | ResolvedDependencyKind::ChecksumEvidence
    );
    if edge.precision == TopologyPrecision::ExactLockfile && !exact_lockfile_allowed {
        diagnostics.push(topology_diagnostic(
            FactFamily::ResolvedDependencyEdge,
            db.resolve_stable_key(edge.stable_key).as_ref(),
            "ResolvedDependencyEdgeFact.precision",
            "exact_lockfile_requires_lockfile_or_checksum_row",
        ));
    }
    check_topology_precision(
        diagnostics,
        FactFamily::ResolvedDependencyEdge,
        db.resolve_stable_key(edge.stable_key).as_ref(),
        "ResolvedDependencyEdgeFact.precision",
        edge.precision,
        edge.status,
        exact_lockfile_allowed,
    );
}

fn check_import_to_package_precision(
    db: &AnalysisDb,
    diagnostics: &mut Vec<Diagnostic>,
    edge: &crate::module_graph::topology::ImportToPackageFact,
) {
    if matches!(
        edge.status,
        ImportToPackageStatus::Dynamic
            | ImportToPackageStatus::Unsupported
            | ImportToPackageStatus::SetupMissing
    ) && matches!(
        edge.precision,
        TopologyPrecision::ExactStatic | TopologyPrecision::ExactLockfile
    ) {
        diagnostics.push(topology_diagnostic(
            FactFamily::ImportToPackage,
            db.resolve_stable_key(edge.stable_key).as_ref(),
            "ImportToPackageFact.precision",
            "unsupported_or_dynamic_exactness",
        ));
    }
    if edge.precision == TopologyPrecision::ExactLockfile {
        diagnostics.push(topology_diagnostic(
            FactFamily::ImportToPackage,
            db.resolve_stable_key(edge.stable_key).as_ref(),
            "ImportToPackageFact.precision",
            "exact_lockfile_requires_lockfile_or_checksum_row",
        ));
    }
}

fn check_topology_precision(
    diagnostics: &mut Vec<Diagnostic>,
    family: FactFamily,
    stable_key: &str,
    field: &'static str,
    precision: TopologyPrecision,
    status: TopologyStatus,
    exact_lockfile_allowed: bool,
) {
    if precision == TopologyPrecision::ExactLockfile && !exact_lockfile_allowed {
        diagnostics.push(topology_diagnostic(
            family,
            stable_key,
            field,
            "exact_lockfile_requires_lockfile_or_checksum_row",
        ));
    }
    if status == TopologyStatus::Unsupported
        && matches!(
            precision,
            TopologyPrecision::ExactStatic | TopologyPrecision::ExactLockfile
        )
    {
        diagnostics.push(topology_diagnostic(
            family,
            stable_key,
            field,
            "unsupported_or_dynamic_exactness",
        ));
    }
}

fn declared_requirement_exists(
    edge: &crate::module_graph::topology::ImportToPackageFact,
    requirements: &[crate::module_graph::topology::DependencyRequirementFact],
) -> bool {
    let target = external_package_name(&edge.import_path);
    requirements.iter().any(|requirement| {
        requirement.target_name == target
            && edge
                .from_package
                .is_none_or(|package| requirement.from_package == Some(package))
    })
}

fn external_package_name(path: &str) -> &str {
    if let Some(stripped) = path.strip_prefix('@') {
        let mut parts = stripped.split('/');
        let Some(scope_name) = parts.next() else {
            return path;
        };
        let Some(package_name) = parts.next() else {
            return path;
        };
        let end = 1 + scope_name.len() + 1 + package_name.len();
        &path[..end]
    } else {
        path.split('/').next().unwrap_or(path)
    }
}

fn topology_diagnostic(
    family: FactFamily,
    stable_key: &str,
    field: &'static str,
    reason: impl Into<String>,
) -> Diagnostic {
    internal_diagnostic(format!(
        "Topology validation failed for {} stable key.",
        family.label()
    ))
    .with_evidence("family", family.label())
    .with_evidence("stable_key", stable_key.to_string())
    .with_evidence("field", field)
    .with_evidence("reason", reason.into())
}

fn type_value_alias_diagnostic(
    _family: FactFamily,
    stable_key: &str,
    field: &'static str,
    reason: impl Into<String>,
) -> Diagnostic {
    internal_diagnostic("Internal analysis validation failed.")
        .with_evidence("component", "analysis")
        .with_evidence("stable_key", stable_key.to_string())
        .with_evidence("field", field)
        .with_evidence("reason", reason.into())
}

fn is_semantic_fact_family(family: FactFamily) -> bool {
    matches!(
        family,
        FactFamily::Scope
            | FactFamily::SemanticImport
            | FactFamily::Export
            | FactFamily::Alias
            | FactFamily::Resolution
            | FactFamily::GeneratedSymbol
            | FactFamily::StableExport
    )
}

fn semantic_reference_keys(db: &AnalysisDb, ids: &IdSets) -> BTreeSet<String> {
    let mut keys = ids.symbol_stable_keys.clone();
    keys.extend(ids.reference_stable_keys.iter().cloned());
    for scope in db.scopes() {
        keys.insert(db.resolve_stable_key(scope.stable_key).to_string());
    }
    for import in db.semantic_imports() {
        keys.insert(db.resolve_stable_key(import.stable_key).to_string());
    }
    for export in db.exports() {
        keys.insert(db.resolve_stable_key(export.stable_key).to_string());
    }
    for alias in db.aliases() {
        keys.insert(db.resolve_stable_key(alias.stable_key).to_string());
    }
    for resolution in db.resolution_facts() {
        keys.insert(db.resolve_stable_key(resolution.stable_key).to_string());
    }
    for generated in db.generated_symbols() {
        keys.insert(db.resolve_stable_key(generated.stable_key).to_string());
    }
    for stable_export in db.stable_exports() {
        keys.insert(db.resolve_stable_key(stable_export.stable_key).to_string());
    }
    keys
}

fn semantic_status_allows_missing_references(status: SemanticStatus) -> bool {
    matches!(
        status,
        SemanticStatus::Generated
            | SemanticStatus::External
            | SemanticStatus::Unresolved
            | SemanticStatus::Dynamic
            | SemanticStatus::SetupMissing
            | SemanticStatus::Unsupported
    )
}

fn check_semantic_ref<T>(
    diagnostics: &mut Vec<Diagnostic>,
    valid_ids: &BTreeSet<T>,
    family: FactFamily,
    stable_key: &str,
    field: &'static str,
    value: T,
) where
    T: Copy + Debug + Ord,
{
    if valid_ids.contains(&value) {
        return;
    }
    diagnostics.push(semantic_diagnostic(
        family,
        stable_key,
        format!("{field} does not exist: {value:?}"),
    ));
}

fn check_semantic_optional_ref<T>(
    diagnostics: &mut Vec<Diagnostic>,
    valid_ids: &BTreeSet<T>,
    family: FactFamily,
    stable_key: &str,
    field: &'static str,
    value: Option<T>,
) where
    T: Copy + Debug + Ord,
{
    let Some(value) = value else {
        return;
    };
    check_semantic_ref(diagnostics, valid_ids, family, stable_key, field, value);
}

fn check_semantic_key_ref(
    diagnostics: &mut Vec<Diagnostic>,
    valid_keys: &BTreeSet<String>,
    family: FactFamily,
    stable_key: &str,
    field: &'static str,
    value: &str,
) {
    if !value.is_empty() && valid_keys.contains(value) {
        return;
    }
    diagnostics.push(semantic_diagnostic(
        family,
        stable_key,
        format!("{field} does not exist: {value}"),
    ));
}

fn semantic_diagnostic(
    family: FactFamily,
    stable_key: &str,
    reason: impl Into<String>,
) -> Diagnostic {
    let (family_label, stable_key_label, reason_label) = SEMANTIC_EVIDENCE_ORDER;
    internal_diagnostic(format!(
        "Semantic index validation failed for {} stable key.",
        family.label()
    ))
    .with_evidence(family_label, family.label())
    .with_evidence(stable_key_label, stable_key.to_string())
    .with_evidence(reason_label, reason.into())
}

fn check_ref<T>(
    diagnostics: &mut Vec<PendingIssue>,
    valid_ids: &BTreeSet<T>,
    family: FactFamily,
    run_id: u64,
    field: &'static str,
    value: T,
) where
    T: Copy + Debug + Ord,
{
    if valid_ids.contains(&value) {
        return;
    }
    let reference = FactRef::new(family, run_id);
    diagnostics.push((
        reference_diagnostic(family, run_id, field, value),
        Attribution::Fact(reference),
    ));
}

fn check_optional_ref<T>(
    diagnostics: &mut Vec<PendingIssue>,
    valid_ids: &BTreeSet<T>,
    family: FactFamily,
    run_id: u64,
    field: &'static str,
    value: Option<T>,
) where
    T: Copy + Debug + Ord,
{
    let Some(value) = value else {
        return;
    };
    check_ref(diagnostics, valid_ids, family, run_id, field, value);
}

fn reference_diagnostic<T: Debug>(
    family: FactFamily,
    run_id: u64,
    field: &'static str,
    value: T,
) -> Diagnostic {
    internal_diagnostic(format!(
        "Fact metadata reference validation failed for {}#{}.",
        family.label(),
        run_id
    ))
    .with_evidence("family", family.label())
    .with_evidence("fact_ref", fact_ref_value(FactRef::new(family, run_id)))
    .with_evidence("field", field)
    .with_evidence("value", format!("{value:?}"))
}

fn provider_manifest_diagnostic(
    reference: FactRef,
    field: &'static str,
    value: &'static str,
) -> Diagnostic {
    internal_diagnostic(format!(
        "Fact metadata provider manifest missing for {}#{}.",
        reference.family.label(),
        reference.run_id
    ))
    .with_evidence("family", reference.family.label())
    .with_evidence("fact_ref", fact_ref_value(reference))
    .with_evidence("field", field)
    .with_evidence("value", value)
}

struct SpanCheck<'a> {
    family: FactFamily,
    run_id: u64,
    field: &'static str,
    owner_file: Option<FileId>,
    span: &'a Span,
}

fn check_span(
    db: &AnalysisDb,
    file_ids: &BTreeSet<FileId>,
    diagnostics: &mut Vec<PendingIssue>,
    check: SpanCheck<'_>,
) {
    let Some(reason) = span_failure_reason(db, file_ids, check.owner_file, check.span) else {
        return;
    };
    let reference = FactRef::new(check.family, check.run_id);
    diagnostics.push((
        internal_diagnostic(format!(
            "Fact metadata span validation failed for {}#{}.",
            check.family.label(),
            check.run_id
        ))
        .with_evidence("family", check.family.label())
        .with_evidence(
            "fact_ref",
            fact_ref_value(FactRef::new(check.family, check.run_id)),
        )
        .with_evidence("field", check.field)
        .with_evidence("reason", reason)
        .with_evidence("span_file", format!("{:?}", check.span.file))
        .with_evidence("owner_file", owner_file_value(check.owner_file))
        .with_evidence("start_byte", check.span.start_byte.to_string())
        .with_evidence("end_byte", check.span.end_byte.to_string()),
        Attribution::Fact(reference),
    ));
}

fn span_failure_reason(
    db: &AnalysisDb,
    file_ids: &BTreeSet<FileId>,
    owner_file: Option<FileId>,
    span: &Span,
) -> Option<String> {
    if !file_ids.contains(&span.file) {
        return Some("span file does not exist".to_string());
    }
    if let Some(owner_file) = owner_file
        && owner_file != span.file
    {
        return Some("span file does not match owning file".to_string());
    }
    if span.start_byte > span.end_byte {
        return Some("start_byte exceeds end_byte".to_string());
    }
    let source_len = db.file(span.file).map(|file| file.source.len() as u32)?;
    if span.end_byte > source_len {
        return Some(format!("end_byte exceeds source length {source_len}"));
    }
    None
}

fn precision_within_ceiling(precision: FactPrecision, ceiling: PrecisionCeiling) -> bool {
    match ceiling {
        PrecisionCeiling::Exact => true,
        PrecisionCeiling::Syntax => matches!(
            precision,
            FactPrecision::Syntax
                | FactPrecision::Heuristic
                | FactPrecision::Unresolved
                | FactPrecision::Ambiguous
                | FactPrecision::SetupMissing
                | FactPrecision::Unsupported
        ),
        PrecisionCeiling::SetupAware => !matches!(precision, FactPrecision::Exact),
    }
}

fn internal_diagnostic(message: impl Into<String>) -> Diagnostic {
    Diagnostic::error(
        "polint/internal",
        "<workspace>",
        TextRange::point(1, 1),
        message,
    )
}

fn fact_ref_value(reference: FactRef) -> String {
    format!("{}#{}", reference.family.label(), reference.run_id)
}

fn owner_file_value(owner_file: Option<FileId>) -> String {
    owner_file.map_or_else(|| "none".to_string(), |file| format!("{file:?}"))
}

fn precision_label(precision: FactPrecision) -> &'static str {
    match precision {
        FactPrecision::Exact => "exact",
        FactPrecision::Syntax => "syntax",
        FactPrecision::SetupAware => "setup_aware",
        FactPrecision::Heuristic => "heuristic",
        FactPrecision::Unresolved => "unresolved",
        FactPrecision::Ambiguous => "ambiguous",
        FactPrecision::SetupMissing => "setup_missing",
        FactPrecision::Unsupported => "unsupported",
    }
}

fn ceiling_label(ceiling: PrecisionCeiling) -> &'static str {
    match ceiling {
        PrecisionCeiling::Exact => "exact",
        PrecisionCeiling::Syntax => "syntax",
        PrecisionCeiling::SetupAware => "setup_aware",
    }
}

fn diagnostic_order(left: &Diagnostic, right: &Diagnostic) -> std::cmp::Ordering {
    (
        left.rule_id.as_str(),
        left.file.as_str(),
        left.range.start_line,
        left.range.start_col,
        left.message.as_str(),
        evidence_order_key(left),
        left.stable_fingerprint.as_str(),
    )
        .cmp(&(
            right.rule_id.as_str(),
            right.file.as_str(),
            right.range.start_line,
            right.range.start_col,
            right.message.as_str(),
            evidence_order_key(right),
            right.stable_fingerprint.as_str(),
        ))
}

fn evidence_order_key(diagnostic: &Diagnostic) -> String {
    diagnostic
        .evidence
        .iter()
        .map(|evidence| format!("{}={}", evidence.label, evidence.value))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::validate_fact_metadata;
    use crate::analysis::evidence::facts::{
        EvidenceConfidence, EvidenceNodeFact, EvidenceNodeKind, EvidencePrecision,
        EvidenceProvenance, EvidenceStatus, EvidenceValidation,
    };
    use crate::analysis::evidence::store::EvidenceOutput;
    use crate::analysis::ids::EvidenceNodeId;
    use crate::analysis_kernel::{
        AnalysisKernel, FactConfidence, FactFamily, FactMeta, FactPrecision, FactRef,
        ValidationStatus,
    };
    use crate::core::{
        AnalysisDb, BranchId, BranchObligation, ComplexityMetricFact, FileId, FileMetricFact,
        FunctionFact, FunctionId, FunctionMetricFact, Language, ModuleNode, ModuleNodeId,
        ModuleNodeKind, PackageId, Span, TestFact, TsComponentFact,
    };
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    #[test]
    fn metadata_validation_conflict_records_render_internal_diagnostics_with_evidence() {
        let mut db = AnalysisDb::new();
        let existing = FactRef::new(FactFamily::Import, 1);
        let incoming = FactRef::new(FactFamily::Import, 2);

        let meta = test_meta(&db, FactFamily::Import, "import:key", "payload:a");
        db.fact_meta_mut_for_test().insert(existing, meta);
        let meta = test_meta(&db, FactFamily::Import, "import:key", "payload:b");
        db.fact_meta_mut_for_test().insert(incoming, meta);

        let diagnostics = validate_fact_metadata(&db, AnalysisKernel::provider_manifests());

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule_id, "polint/internal");
        assert!(
            diagnostics[0]
                .message
                .starts_with("Fact metadata stable key conflict detected")
        );
        assert_eq!(
            evidence_labels(&diagnostics[0]),
            BTreeSet::from(["existing_ref", "family", "incoming_ref", "stable_key",])
        );
    }

    #[test]
    fn metadata_validation_span_failures_are_reported_deterministically() {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/app.ts"),
            "src/app.ts".to_string(),
            "abc".to_string(),
        );
        let other_file = db.add_file(
            PathBuf::from("src/other.ts"),
            "src/other.ts".to_string(),
            "abcdef".to_string(),
        );
        db.push_function(FunctionFact::new(
            FunctionId::from_raw(99),
            file,
            "too_long".to_string(),
            span(file, 0, 4),
            Language::TypeScript,
            false,
            false,
            1,
            Vec::new(),
        ));
        db.push_function(FunctionFact::new(
            FunctionId::from_raw(99),
            file,
            "reversed".to_string(),
            span(file, 2, 1),
            Language::TypeScript,
            false,
            false,
            1,
            Vec::new(),
        ));
        db.push_function(FunctionFact::new(
            FunctionId::from_raw(99),
            file,
            "wrong_file".to_string(),
            span(other_file, 0, 1),
            Language::TypeScript,
            false,
            false,
            1,
            Vec::new(),
        ));

        let diagnostics = validate_fact_metadata(&db, AnalysisKernel::provider_manifests());
        let messages = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>();

        assert_eq!(messages.len(), 3);
        assert!(
            messages
                .iter()
                .all(|message| message.starts_with("Fact metadata span validation failed"))
        );
    }

    #[test]
    fn metadata_validation_reference_failures_cover_current_focused_fields() {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/app.tsx"),
            "src/app.tsx".to_string(),
            "export function Button() { return null; }\n".to_string(),
        );
        db.push_branch(BranchObligation::new(
            BranchId::from_raw(99),
            Some(FunctionId::from_raw(404)),
            file,
            span(file, 0, 1),
            "enabled".to_string(),
            "true".to_string(),
            false,
            "branch:key".to_string(),
        ));
        db.push_test(TestFact::new(
            file,
            Some(FunctionId::from_raw(405)),
            "TestButton".to_string(),
            span(file, 0, 1),
            Vec::new(),
            0,
            0,
            Vec::new(),
            0,
        ));
        db.push_ts_component(TsComponentFact::new(
            file,
            Some(FunctionId::from_raw(406)),
            "Button".to_string(),
            span(file, 0, 1),
        ));
        db.replace_module_graph_facts(
            Vec::new(),
            vec![ModuleNode::new(
                ModuleNodeId::from_raw(99),
                ModuleNodeKind::File,
                "missing".to_string(),
                Some(FileId::from_raw(404)),
                Some(PackageId::from_raw(405)),
                Some(Language::Tsx),
            )],
            Vec::new(),
        );
        db.replace_metric_facts(
            vec![FileMetricFact::new(
                FileId::from_raw(406),
                Language::Tsx,
                1,
                1,
                1,
                0,
            )],
            vec![FunctionMetricFact::new(
                FunctionId::from_raw(407),
                FileId::from_raw(407),
                "Button".to_string(),
                span(file, 0, 1),
                Language::Tsx,
                1,
                1,
            )],
            vec![ComplexityMetricFact::new(
                FunctionId::from_raw(408),
                FileId::from_raw(408),
                "Button".to_string(),
                span(file, 0, 1),
                Language::Tsx,
                1,
            )],
        );

        let diagnostics = validate_fact_metadata(&db, AnalysisKernel::provider_manifests());
        let evidence_values = diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic
                    .message
                    .starts_with("Fact metadata reference validation failed")
            })
            .flat_map(|diagnostic| {
                diagnostic
                    .evidence
                    .iter()
                    .filter(|evidence| evidence.label == "field")
                    .map(|evidence| evidence.value.as_str())
            })
            .collect::<BTreeSet<_>>();

        assert_eq!(
            evidence_values,
            BTreeSet::from([
                "BranchObligation.function",
                "ComplexityMetricFact.file",
                "ComplexityMetricFact.function",
                "FileMetricFact.file",
                "FunctionMetricFact.file",
                "FunctionMetricFact.function",
                "ModuleNode.file",
                "ModuleNode.package",
                "TestFact.function",
                "TsComponentFact.function",
            ])
        );
    }

    #[test]
    fn metadata_validation_reports_evidence_external_reference_failures() {
        let mut db = AnalysisDb::new();
        db.replace_evidence_facts(EvidenceOutput {
            nodes: vec![evidence_node(0, FileId::from_raw(404))],
            ..EvidenceOutput::empty()
        })
        .expect("evidence store accepts external refs for kernel validation");

        let diagnostics = validate_fact_metadata(&db, AnalysisKernel::provider_manifests());
        let evidence_values = diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic
                    .message
                    .starts_with("Fact metadata reference validation failed")
            })
            .flat_map(|diagnostic| {
                diagnostic
                    .evidence
                    .iter()
                    .filter(|evidence| evidence.label == "field")
                    .map(|evidence| evidence.value.as_str())
            })
            .collect::<BTreeSet<_>>();

        assert!(evidence_values.contains("EvidenceNodeFact.file"));
    }

    #[test]
    fn metadata_validation_reports_evidence_span_failures() {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/app.ts"),
            "src/app.ts".to_string(),
            "abc".to_string(),
        );
        let mut node = evidence_node(0, file);
        node.span = Some(span(file, 1, 4));
        db.replace_evidence_facts(EvidenceOutput {
            nodes: vec![node],
            ..EvidenceOutput::empty()
        })
        .expect("evidence store accepts spans for kernel validation");

        let diagnostics = validate_fact_metadata(&db, AnalysisKernel::provider_manifests());

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .starts_with("Fact metadata span validation failed")
                && diagnostic.evidence.iter().any(|evidence| {
                    evidence.label == "field" && evidence.value == "EvidenceNodeFact.span"
                })
        }));
    }

    #[test]
    fn metadata_validation_precision_ceiling_violations_name_provider_family_and_precision() {
        let mut db = AnalysisDb::new();
        let stable_key = db.stable_key_interner().intern("metric:key");
        db.fact_meta_mut_for_test().insert(
            FactRef::new(FactFamily::FileMetric, 0),
            FactMeta {
                stable_key,
                producer_id: "polint.metrics",
                layer_id: "polint.metrics",
                precision: FactPrecision::Exact,
                confidence: FactConfidence::High,
                validation: ValidationStatus::NativeTrusted,
                payload_digest: "payload:a".to_string(),
            },
        );

        let mut report = validate_fact_metadata(&db, AnalysisKernel::provider_manifests());

        assert_eq!(report.len(), 1);
        assert!(
            report[0]
                .message
                .starts_with("Fact metadata precision ceiling violated")
        );
        assert_eq!(
            evidence_labels(&report[0]),
            BTreeSet::from(["ceiling", "family", "precision", "producer_id"])
        );
        assert_eq!(report.issues[0].fact_family, Some(FactFamily::FileMetric));
        assert_eq!(report.issues[0].provider_ids, ["polint.metrics"]);
        let expected = report.downgrades();
        report.issues[0].reason = "different rendering".to_string();
        report.issues[0].evidence.clear();
        report.issues[0].presentation.stable_fingerprint = "different fingerprint".to_string();
        assert_eq!(report.downgrades(), expected);
        let global = super::ValidationIssue::from_pending(
            (
                super::internal_diagnostic("global"),
                super::Attribution::Global,
            ),
            &AnalysisDb::new(),
            &Default::default(),
        );
        assert_eq!(global.fact_family, None);
        assert!(global.provider_ids.is_empty());
        assert!(
            super::ValidationReport {
                diagnostics: vec![global.render()],
                issues: vec![global],
            }
            .downgrades()
            .contains("unrelated")
        );
    }

    /// A family-scoped issue must downgrade only the providers that emitted
    /// facts in that family. Escalating to a global downgrade would fail every
    /// provider, which blocks every capability-requesting rule and silently
    /// empties the run of findings.
    #[test]
    fn family_scoped_issues_downgrade_only_that_familys_producers() {
        let mut db = AnalysisDb::new();
        let stable_key = db.stable_key_interner().intern("metric:key");
        db.fact_meta_mut_for_test().insert(
            FactRef::new(FactFamily::FileMetric, 0),
            FactMeta {
                stable_key,
                producer_id: "polint.metrics",
                layer_id: "polint.metrics",
                precision: FactPrecision::Syntax,
                confidence: FactConfidence::High,
                validation: ValidationStatus::NativeTrusted,
                payload_digest: "payload:a".to_string(),
            },
        );

        for attribution in [
            super::Attribution::Family(FactFamily::FileMetric),
            // A fact whose own metadata row is absent still resolves through
            // its family rather than escalating.
            super::Attribution::Fact(FactRef::new(FactFamily::FileMetric, 404)),
        ] {
            let issue = super::ValidationIssue::from_pending(
                (super::internal_diagnostic("family scoped"), attribution),
                &db,
                &AnalysisKernel::provider_manifests()
                    .iter()
                    .map(|manifest| (manifest.id, *manifest))
                    .collect(),
            );
            assert_eq!(issue.provider_ids, ["polint.metrics"], "{attribution:?}");
            let downgrades = super::ValidationReport {
                diagnostics: vec![issue.render()],
                issues: vec![issue],
            }
            .downgrades();
            assert!(downgrades.contains("polint.metrics"), "{attribution:?}");
            assert!(!downgrades.contains("polint.go.syntax"), "{attribution:?}");
        }
    }

    /// TypeScript declaration merging (two `export interface Foo`) emits one
    /// `Export` fact per declaration for a single merged entity, so they share
    /// a stable key. That is an identity artefact, not broken output: it must
    /// be reported without downgrading `polint.symbol_graph`, because a
    /// downgrade blocks every rule requesting `symbols` or `references`.
    #[test]
    fn stable_key_conflicts_are_reported_without_downgrading_the_producer() {
        let mut db = AnalysisDb::new();
        let stable_key = db
            .stable_key_interner()
            .intern("Export|export_name=MergeMe");
        for run_id in [0, 1] {
            db.fact_meta_mut_for_test().insert(
                FactRef::new(FactFamily::Export, run_id),
                FactMeta {
                    stable_key,
                    producer_id: super::SYMBOL_GRAPH_PROVIDER_ID,
                    layer_id: super::SYMBOL_GRAPH_PROVIDER_ID,
                    precision: FactPrecision::Exact,
                    confidence: FactConfidence::High,
                    validation: ValidationStatus::NativeTrusted,
                    payload_digest: format!("payload:{run_id}"),
                },
            );
        }

        let report = validate_fact_metadata(&db, AnalysisKernel::provider_manifests());

        let conflict = report
            .issues
            .iter()
            .find(|issue| issue.reason.contains("stable key conflict"))
            .expect("the duplicate stable key is still reported");
        assert_eq!(conflict.fact_family, Some(FactFamily::Export));
        assert_eq!(conflict.provider_ids, [super::SYMBOL_GRAPH_PROVIDER_ID]);
        assert!(
            report
                .iter()
                .any(|diagnostic| diagnostic.message.contains("stable key conflict")),
            "the conflict stays user-visible as an internal diagnostic"
        );
        // The synthetic metadata rows above trip other validators too, so the
        // no-downgrade property is asserted on the conflict issue alone.
        let conflict_only = super::ValidationReport {
            diagnostics: vec![conflict.render()],
            issues: vec![conflict.clone()],
        };
        assert!(
            !conflict_only
                .downgrades()
                .contains(super::SYMBOL_GRAPH_PROVIDER_ID),
            "an identity-only conflict must not reject the symbol graph provider"
        );
        assert_eq!(
            conflict_only.downgrades(),
            crate::analysis_kernel::ValidationDowngrades::default(),
            "an identity-only conflict must not downgrade anything, globally or otherwise"
        );
    }

    /// An extension-produced fact names a producer that is not in the static
    /// manifest inventory. The known co-owner must survive rather than the
    /// whole attribution collapsing into a global downgrade.
    #[test]
    fn unknown_producer_ids_do_not_erase_a_known_co_owner() {
        let mut db = AnalysisDb::new();
        let stable_key = db.stable_key_interner().intern("metric:key");
        db.fact_meta_mut_for_test().insert(
            FactRef::new(FactFamily::FileMetric, 0),
            FactMeta {
                stable_key,
                producer_id: "acme.extension",
                layer_id: "polint.metrics",
                precision: FactPrecision::Syntax,
                confidence: FactConfidence::High,
                validation: ValidationStatus::NativeTrusted,
                payload_digest: "payload:a".to_string(),
            },
        );
        let issue = super::ValidationIssue::from_pending(
            (
                super::internal_diagnostic("extension fact"),
                super::Attribution::Fact(FactRef::new(FactFamily::FileMetric, 0)),
            ),
            &db,
            &AnalysisKernel::provider_manifests()
                .iter()
                .map(|manifest| (manifest.id, *manifest))
                .collect(),
        );
        assert_eq!(issue.provider_ids, ["polint.metrics"]);
        let downgrades = super::ValidationReport {
            diagnostics: vec![issue.render()],
            issues: vec![issue],
        }
        .downgrades();
        assert!(downgrades.contains("polint.metrics"));
        assert!(!downgrades.contains("polint.source"));
    }

    #[test]
    fn metadata_validation_reports_unknown_producer_and_layer_ids() {
        let mut db = AnalysisDb::new();
        let stable_key = db.stable_key_interner().intern("metric:key");
        db.fact_meta_mut_for_test().insert(
            FactRef::new(FactFamily::FileMetric, 0),
            FactMeta {
                stable_key,
                producer_id: "polint.unknown_producer",
                layer_id: "polint.unknown_layer",
                precision: FactPrecision::Syntax,
                confidence: FactConfidence::High,
                validation: ValidationStatus::NativeTrusted,
                payload_digest: "payload:a".to_string(),
            },
        );

        let diagnostics = validate_fact_metadata(&db, AnalysisKernel::provider_manifests());
        let provider_fields = diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic
                    .message
                    .starts_with("Fact metadata provider manifest missing")
            })
            .flat_map(|diagnostic| {
                diagnostic
                    .evidence
                    .iter()
                    .filter(|evidence| evidence.label == "field")
                    .map(|evidence| evidence.value.as_str())
            })
            .collect::<BTreeSet<_>>();

        assert_eq!(provider_fields, BTreeSet::from(["layer_id", "producer_id"]));
    }

    #[test]
    fn metadata_validation_accepts_known_extension_producer_and_rejects_exact_without_evidence() {
        let mut db = AnalysisDb::new();
        db.replace_extension_facts(crate::analysis::extensions::store::ExtensionOutput {
            activations: Vec::new(),
            accepted: vec![crate::analysis::extensions::store::AcceptedExtensionFact {
                extension_id: "demo".to_string(),
                provider_id: "routes".to_string(),
                fact_family: "extension.routes".to_string(),
                stable_key: crate::core::stable_key_for_test("route:/a"),
                binding_refs: Vec::new(),
                precision: crate::analysis::extensions::sinks::ExtensionFactPrecision::Exact,
                confidence: crate::analysis::extensions::sinks::ExtensionFactConfidence::High,
                status: crate::analysis::extensions::sinks::ExtensionFactStatus::Accepted,
                evidence: vec!["fixture".to_string()],
                payload_labels: Vec::new(),
                payload_digest: "payload".to_string(),
            }],
            rejected: Vec::new(),
        });

        let diagnostics = validate_fact_metadata(&db, AnalysisKernel::provider_manifests());

        assert!(!diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .starts_with("Fact metadata provider manifest missing")
        }));
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .starts_with("Fact metadata precision ceiling violated")
                && diagnostic.evidence.iter().any(|evidence| {
                    evidence.label == "ceiling"
                        && evidence.value == "extension_exact_requires_validation_evidence"
                })
        }));
    }

    fn test_meta(
        db: &AnalysisDb,
        family: FactFamily,
        stable_key: &str,
        payload_digest: &str,
    ) -> FactMeta {
        FactMeta {
            stable_key: db.stable_key_interner().intern(stable_key),
            producer_id: match family {
                FactFamily::SourceFile => "polint.source",
                FactFamily::FileMetric => "polint.metrics",
                _ => "polint.go.syntax",
            },
            layer_id: "polint.go.syntax",
            precision: FactPrecision::Syntax,
            confidence: FactConfidence::High,
            validation: ValidationStatus::NativeTrusted,
            payload_digest: payload_digest.to_string(),
        }
    }

    fn span(file: FileId, start_byte: u32, end_byte: u32) -> Span {
        Span::new(
            file,
            start_byte,
            end_byte,
            1,
            start_byte + 1,
            1,
            end_byte + 1,
        )
    }

    fn evidence_node(id: u64, file: FileId) -> EvidenceNodeFact {
        EvidenceNodeFact {
            id: EvidenceNodeId(id),
            kind: EvidenceNodeKind::Synthetic,
            language: Language::TypeScript,
            file: Some(file),
            function: None,
            body: None,
            operation: None,
            cfg_node: None,
            place: None,
            symbol: None,
            reference: None,
            call_site: None,
            span: None,
            status: EvidenceStatus::Present,
            precision: EvidencePrecision::Syntax,
            provenance: EvidenceProvenance::Native,
            validation: EvidenceValidation::Native,
            confidence: EvidenceConfidence::High,
            compact_label: None,
            source_fact_stable_keys: Vec::new(),
            stable_key: crate::core::stable_key_for_test(&format!("evidence:node:{id}")),
        }
    }

    fn evidence_labels(diagnostic: &crate::diagnostics::Diagnostic) -> BTreeSet<&str> {
        diagnostic
            .evidence
            .iter()
            .map(|evidence| evidence.label.as_str())
            .collect()
    }
}

//! Population pass for `polint.semantic_graph`.
//!
//! [`build_semantic_graph`] projects a real-but-minimal semantic graph from the
//! already-available v1.2 fact families — functions, packages, scopes, call sites,
//! and values — emitting nodes, `Call`/`MemberOf` edges, and
//! `CopyEdge`/`CallConstraint` constraints, plus the private TS object-model
//! rows as `Alloc`/`FieldStore`/`FieldLoad`/receiver constraints and accepted
//! adaptation-model facts as `ModelEdge` constraints. It is a
//! **read-only projection**: it reads `db` and returns a fresh
//! [`SemanticGraphOutput`] without mutating upstream fact families. All
//! node, edge, and constraint stable keys are composed from referenced stable
//! identities through the length-prefixed recipe, never run-local dense IDs; dense
//! IDs are assigned later by `normalized()`.
//!
//! `TypeConstraint` is intentionally not emitted by this minimal pass.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[cfg(feature = "lang-typescript")]
use oxc_allocator::Allocator;
#[cfg(all(test, feature = "lang-go", feature = "lang-typescript"))]
use oxc_semantic::SemanticBuilder;

use crate::analysis_api::FactFamily;
#[cfg(all(test, feature = "lang-go", feature = "lang-typescript"))]
use crate::analysis_api::SourceFile;
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::adaptation::store::AdaptationModelStore;
use crate::analysis_neutral::ids::{ObjectTokenId, PlaceId, SemanticNodeId};
use crate::analysis_neutral::semantic_graph::build::{
    function_node_key, node_key_from_identity, place_node_key,
};
use crate::analysis_neutral::semantic_graph::constraints::ConstraintKind;
#[cfg(all(test, feature = "lang-typescript"))]
use crate::analysis_neutral::semantic_graph::facts::SemanticPrecision;
use crate::analysis_neutral::semantic_graph::facts::{EdgeKind, NodeKind};
use crate::analysis_neutral::semantic_graph::store::SemanticGraphOutput;
use crate::analysis_neutral::stable_key::semantic_stable_key;
use crate::internal_core::{FileId, StableKeyInterner};
use crate::ts::binding::direct::{
    TsDirectBindingModuleFile, TsDirectBindingModuleInput, resolve_direct_bindings_with_modules,
};
use crate::ts::binding::facts::{TsDirectBindingFact, TsDirectBindingKind, TsDirectBindingStatus};
use crate::ts::binding::store::TsDirectBindingOutput;
use crate::ts::callable_flow::TsCallableFlow;
#[cfg(all(test, feature = "lang-typescript"))]
use crate::ts::inventory::extract::extract_ts_inventory;
#[cfg(all(test, feature = "lang-go", feature = "lang-typescript"))]
use crate::ts::inventory::extract::{
    extract_ts_inventory_from_program, mark_inventory_partial_ast,
};
use crate::ts::inventory::store::TsInventoryOutput;
use crate::ts::object_model::facts::{
    TsObjectModelStatus, TsPropertyKey, TsPropertyReadFact, TsPropertyWriteFact,
    TsReceiverBindingFact,
};
use crate::ts::object_model::store::TsObjectModelOutput;
#[cfg(feature = "lang-typescript")]
use crate::ts::parse::parse_ts_file;
#[cfg(all(test, feature = "lang-go", feature = "lang-typescript"))]
use crate::ts::scope::extract::{extract_ts_scope_from_program, mark_scope_partial_ast};
use crate::ts::scope::store::TsScopeOutput;
pub(crate) use crate::ts::token_flow::{TsTokenSourceFlow, collect_ts_token_source_flows};

/// Projects a real-but-minimal semantic graph from already-stored v1.2 facts.
///
/// Reads `db` only; the returned [`SemanticGraphOutput`] is the sole new artifact.
/// Dense IDs are not assigned here: every node, edge, and constraint is emitted with
/// the default dense ID and a composed stable key. The caller runs `normalized()` and
/// `SemanticGraphStore::from_output` to sort by stable key and assign dense IDs.
#[cfg(all(test, feature = "lang-typescript"))]
pub(crate) fn build_semantic_graph(db: &impl AnalysisHost) -> SemanticGraphOutput {
    let ts_direct_bindings = collect_ts_direct_binding_collection(db);
    build_semantic_graph_with_ts_direct_binding_collection(
        db,
        &ts_direct_bindings,
        no_additional_projection,
    )
}

pub(crate) fn build_semantic_graph_with_ts_direct_binding_collection<H: AnalysisHost>(
    db: &H,
    ts_direct_bindings: &TsDirectBindingCollection,
    project_additional_facts: fn(
        &H,
        &mut crate::analysis_neutral::semantic_graph::build::SemanticGraphBuilder,
    ),
) -> SemanticGraphOutput {
    build_semantic_graph_with_ts_direct_binding_collection_and_adaptation_models(
        db,
        ts_direct_bindings,
        &AdaptationModelStore::default(),
        project_additional_facts,
    )
}

#[cfg(all(test, feature = "lang-typescript"))]
pub(crate) fn build_semantic_graph_with_ts_direct_bindings(
    db: &impl AnalysisHost,
    ts_direct_bindings: &[TsDirectBindingFact],
) -> SemanticGraphOutput {
    build_semantic_graph_with_ts_direct_bindings_and_adaptation_models(
        db,
        ts_direct_bindings,
        &AdaptationModelStore::default(),
    )
}

#[cfg(all(test, feature = "lang-typescript"))]
pub(crate) fn build_semantic_graph_with_ts_direct_bindings_and_adaptation_models(
    db: &impl AnalysisHost,
    ts_direct_bindings: &[TsDirectBindingFact],
    adaptation_models: &AdaptationModelStore,
) -> SemanticGraphOutput {
    build_semantic_graph_with_inputs(
        db,
        ts_direct_bindings,
        adaptation_models,
        &TsObjectModelOutput::default(),
        None,
        &[],
        no_additional_projection,
    )
}

pub(crate) fn build_semantic_graph_with_ts_direct_binding_collection_and_adaptation_models<
    H: AnalysisHost,
>(
    db: &H,
    ts_direct_bindings: &TsDirectBindingCollection,
    adaptation_models: &AdaptationModelStore,
    project_additional_facts: fn(
        &H,
        &mut crate::analysis_neutral::semantic_graph::build::SemanticGraphBuilder,
    ),
) -> SemanticGraphOutput {
    let object_model = ts_direct_bindings.object_model_output();
    build_semantic_graph_with_inputs(
        db,
        ts_direct_bindings.bindings(),
        adaptation_models,
        &object_model,
        Some(ts_direct_bindings.analyses.as_slice()),
        &ts_direct_bindings.callable_flows,
        project_additional_facts,
    )
}

fn build_semantic_graph_with_inputs<H: AnalysisHost>(
    db: &H,
    ts_direct_bindings: &[TsDirectBindingFact],
    adaptation_models: &AdaptationModelStore,
    object_model: &TsObjectModelOutput,
    ts_analyses: Option<&[TsFileAnalysis]>,
    callable_flows: &[TsCallableFlow],
    project_additional_facts: fn(
        &H,
        &mut crate::analysis_neutral::semantic_graph::build::SemanticGraphBuilder,
    ),
) -> SemanticGraphOutput {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let mut builder = GraphBuilder::default();

    builder.project_nodes(db);
    builder.project_call_edges_and_constraints(db);
    project_additional_facts(db, &mut builder.inner);
    builder.project_ts_direct_bindings(db, ts_direct_bindings);
    builder.project_ts_token_source_flows(db, ts_analyses);
    builder.project_ts_callable_flows(db, callable_flows);
    builder.project_ts_object_model(db, object_model);
    builder.project_adaptation_models(interner, adaptation_models);
    builder.project_member_of_edges(db);
    builder.project_value_constraints(db);
    // TypeConstraint intentionally emits zero rows in this pass (see module docs).
    // The base places for access-path projections are already projected as nodes
    // above.

    builder.into_output()
}

#[cfg(all(test, feature = "lang-typescript"))]
fn no_additional_projection(
    _db: &impl AnalysisHost,
    _builder: &mut crate::analysis_neutral::semantic_graph::build::SemanticGraphBuilder,
) {
}

pub(crate) use crate::ts::semantic_graph::TsFileAnalysis;

#[cfg(all(test, feature = "lang-go", feature = "lang-typescript"))]
struct TsBindingFileAnalysis {
    file: FileId,
    inventory: TsInventoryOutput,
    scope: TsScopeOutput,
}

trait TsDirectBindingAnalysis {
    fn file(&self) -> FileId;
    fn inventory(&self) -> &TsInventoryOutput;
    fn scope(&self) -> &TsScopeOutput;
}

impl TsDirectBindingAnalysis for TsFileAnalysis {
    fn file(&self) -> FileId {
        self.file
    }

    fn inventory(&self) -> &TsInventoryOutput {
        &self.inventory
    }

    fn scope(&self) -> &TsScopeOutput {
        &self.scope
    }
}

#[cfg(all(test, feature = "lang-go", feature = "lang-typescript"))]
impl TsDirectBindingAnalysis for TsBindingFileAnalysis {
    fn file(&self) -> FileId {
        self.file
    }

    fn inventory(&self) -> &TsInventoryOutput {
        &self.inventory
    }

    fn scope(&self) -> &TsScopeOutput {
        &self.scope
    }
}

pub(crate) struct TsDirectBindingCollection {
    output: TsDirectBindingOutput,
    analyses: Vec<TsFileAnalysis>,
    callable_flows: Vec<TsCallableFlow>,
}

impl TsDirectBindingCollection {
    pub(crate) fn output(&self) -> &TsDirectBindingOutput {
        &self.output
    }

    pub(crate) fn bindings(&self) -> &[TsDirectBindingFact] {
        &self.output.bindings
    }

    pub(crate) fn object_model_output(&self) -> TsObjectModelOutput {
        let mut output = TsObjectModelOutput::default();
        for analysis in &self.analyses {
            output
                .allocations
                .extend(analysis.object_model.allocations.iter().cloned());
            output
                .property_writes
                .extend(analysis.object_model.property_writes.iter().cloned());
            output
                .property_reads
                .extend(analysis.object_model.property_reads.iter().cloned());
            output
                .receiver_bindings
                .extend(analysis.object_model.receiver_bindings.iter().cloned());
            output
                .prototype_links
                .extend(analysis.object_model.prototype_links.iter().cloned());
        }
        output
    }
}

pub(crate) fn collect_ts_direct_binding_collection(
    db: &impl AnalysisHost,
) -> TsDirectBindingCollection {
    let (analyses, callable_flows) = collect_ts_file_analyses(db);
    let output = collect_ts_direct_bindings_from_analyses(db, &analyses);
    TsDirectBindingCollection {
        output,
        analyses,
        callable_flows,
    }
}

#[cfg(all(test, feature = "lang-go", feature = "lang-typescript"))]
pub(crate) fn collect_ts_direct_bindings(db: &impl AnalysisHost) -> TsDirectBindingOutput {
    let analyses = collect_ts_binding_file_analyses(db);
    collect_ts_direct_bindings_from_analyses(db, &analyses)
}

#[cfg(feature = "lang-typescript")]
fn collect_ts_file_analyses(db: &impl AnalysisHost) -> (Vec<TsFileAnalysis>, Vec<TsCallableFlow>) {
    let interner = db.stable_key_interner();
    let files = db
        .files()
        .iter()
        .filter(|file| file.language.is_ts_family())
        .collect::<Vec<_>>();
    let arenas = files
        .iter()
        .map(|_| Allocator::default())
        .collect::<Vec<_>>();
    let parsed = files
        .iter()
        .zip(&arenas)
        .map(|(file, arena)| parse_ts_file(arena, file))
        .collect::<Vec<_>>();
    let analyses = files
        .iter()
        .zip(&parsed)
        .map(|(file, parsed)| {
            crate::ts::semantic_graph::analyze_parsed_ts_file(&interner, file, parsed)
        })
        .collect();
    let programs = files
        .iter()
        .zip(&parsed)
        .filter(|(_, parsed)| parsed.fully_parsed)
        .map(|(file, parsed)| (file.id, parsed.program()))
        .collect();
    let callable_flows = crate::ts::callable_flow::collect_callable_flows(db, &programs);
    (analyses, callable_flows)
}

#[cfg(not(feature = "lang-typescript"))]
fn collect_ts_file_analyses(db: &impl AnalysisHost) -> (Vec<TsFileAnalysis>, Vec<TsCallableFlow>) {
    let interner = db.stable_key_interner();
    let analyses = db
        .files()
        .iter()
        .filter(|file| file.language.is_ts_family())
        .map(|file| crate::ts::semantic_graph::analyze_ts_file(&interner, file))
        .collect();
    (analyses, Vec::new())
}

#[cfg(all(test, feature = "lang-go", feature = "lang-typescript"))]
fn collect_ts_binding_file_analyses(db: &impl AnalysisHost) -> Vec<TsBindingFileAnalysis> {
    let interner = db.stable_key_interner();
    db.files()
        .iter()
        .filter(|file| file.language.is_ts_family())
        .map(|file| analyze_ts_binding_file(&interner, file))
        .collect()
}

#[cfg(all(test, feature = "lang-go", feature = "lang-typescript"))]
fn analyze_ts_binding_file(
    interner: &StableKeyInterner,
    file: &SourceFile,
) -> TsBindingFileAnalysis {
    let source = file.source.as_ref();
    let allocator = Allocator::default();
    let parsed = parse_ts_file(&allocator, file);

    if parsed.is_catastrophic() {
        return TsBindingFileAnalysis {
            file: file.id,
            inventory: TsInventoryOutput::default(),
            scope: TsScopeOutput::default(),
        };
    }

    let semantic = SemanticBuilder::new().build(parsed.program()).semantic;
    let nodes = semantic.nodes();
    let mut inventory =
        extract_ts_inventory_from_program(interner, file, source, parsed.program(), nodes)
            .normalized(interner);
    let mut scope = extract_ts_scope_from_program(
        interner,
        file,
        source,
        parsed.program(),
        semantic.scoping(),
        nodes,
    )
    .normalized(interner);
    if !parsed.fully_parsed {
        mark_inventory_partial_ast(&mut inventory);
        mark_scope_partial_ast(&mut scope);
    }

    TsBindingFileAnalysis {
        file: file.id,
        inventory,
        scope,
    }
}

fn collect_ts_direct_bindings_from_analyses<A: TsDirectBindingAnalysis>(
    db: &impl AnalysisHost,
    analyses: &[A],
) -> TsDirectBindingOutput {
    if analyses.is_empty() {
        return TsDirectBindingOutput::default();
    }

    let analysis_by_file = analyses
        .iter()
        .enumerate()
        .map(|(index, analysis)| (analysis.file(), index))
        .collect::<BTreeMap<_, _>>();
    let module_files = db
        .module_nodes()
        .iter()
        .filter_map(|node| {
            let file = node.file?;
            let analysis = analyses.get(*analysis_by_file.get(&file)?)?;
            Some(TsDirectBindingModuleFile {
                module_node: node.id,
                inventory: analysis.inventory(),
                scope: analysis.scope(),
            })
        })
        .collect::<Vec<_>>();
    let module_input = TsDirectBindingModuleInput {
        imports: db.imports(),
        resolved_imports: db.resolved_imports(),
        module_nodes: db.module_nodes(),
        module_files: &module_files,
    };

    let mut bindings = Vec::new();
    let interner = db.stable_key_interner();
    for analysis in analyses {
        let output = resolve_direct_bindings_with_modules(
            &interner,
            analysis.inventory(),
            analysis.scope(),
            &module_input,
        );
        bindings.extend(output.bindings);
    }

    TsDirectBindingOutput { bindings }.normalized(&interner)
}

/// Accumulates pre-normalization nodes/edges/constraints plus the lookup maps that
/// translate a referenced domain identity to its pre-sort [`SemanticNodeId`]. The
/// pre-sort IDs are just insertion handles; `normalized()` reassigns the persisted
/// dense IDs after the stable-key sort.
#[derive(Default)]
struct GraphBuilder {
    inner: crate::analysis_neutral::semantic_graph::build::SemanticGraphBuilder,
}

impl GraphBuilder {
    fn intern_node(
        &mut self,
        interner: &StableKeyInterner,
        kind: NodeKind,
        stable_key_text: String,
    ) -> SemanticNodeId {
        self.inner.intern_node(interner, kind, stable_key_text)
    }

    fn push_edge(
        &mut self,
        interner: &StableKeyInterner,
        kind: EdgeKind,
        source: SemanticNodeId,
        target: SemanticNodeId,
    ) {
        self.inner.push_edge(interner, kind, source, target);
    }

    fn push_constraint(
        &mut self,
        interner: &StableKeyInterner,
        kind: ConstraintKind,
        identity: &str,
    ) {
        self.inner.push_constraint(interner, kind, identity);
    }

    fn node_for_key(&self, interner: &StableKeyInterner, key: &str) -> Option<SemanticNodeId> {
        self.inner.node_for_key(interner, key)
    }

    fn project_nodes(&mut self, db: &impl AnalysisHost) {
        self.inner.project_nodes(db);
    }

    fn project_call_edges_and_constraints(&mut self, db: &impl AnalysisHost) {
        self.inner.project_call_edges_and_constraints(db);
    }

    fn project_adaptation_models(
        &mut self,
        interner: &StableKeyInterner,
        adaptation_models: &AdaptationModelStore,
    ) {
        self.inner
            .project_adaptation_models(interner, adaptation_models);
    }

    // -- TS direct binding constraints -------------------------------------

    fn project_ts_direct_bindings(
        &mut self,
        db: &impl AnalysisHost,
        bindings: &[TsDirectBindingFact],
    ) {
        let interner_handle = db.stable_key_interner();
        let interner = &interner_handle;
        // Semantic-graph-only projection: TS direct binding rows emit
        // CopyEdge/CallConstraint rows here and do not create CallTargetFact rows.
        // The calls/refined-calls contract remains owned by analysis::calls; direct TS
        // bindings are not part of that fact family.
        let context = TsDirectBindingNodeContext::new(interner, db, self);
        for binding in bindings {
            if binding.status != TsDirectBindingStatus::Resolved {
                continue;
            }
            let Some(callsite_node) =
                context.callsite_node_for_binding(&interner.resolve(binding.callsite_stable_key))
            else {
                continue;
            };
            self.push_constraint(
                interner,
                ConstraintKind::CallConstraint {
                    callsite: callsite_node,
                },
                &interner.resolve(binding.stable_key),
            );

            let Some(target_key) = binding
                .target_function_stable_key
                .map(|key| interner.resolve(key))
            else {
                continue;
            };
            let Some(target_node) = context.function_node_for_binding(&target_key) else {
                continue;
            };
            if direct_binding_emits_copy_edge(binding.kind) {
                self.push_constraint(
                    interner,
                    ConstraintKind::CopyEdge {
                        dst: callsite_node,
                        src: target_node,
                    },
                    &interner.resolve(binding.stable_key),
                );
            }
        }
    }

    fn project_ts_token_source_flows(
        &mut self,
        db: &impl AnalysisHost,
        ts_analyses: Option<&[TsFileAnalysis]>,
    ) {
        let interner_handle = db.stable_key_interner();
        let interner = &interner_handle;
        let context = TsDirectBindingNodeContext::new(interner, db, self);
        if let Some(analyses) = ts_analyses {
            for analysis in analyses {
                for flow in &analysis.token_source_flows {
                    self.project_ts_token_source_flow(interner, flow, &context);
                }
            }
            return;
        }

        for file in db
            .files()
            .iter()
            .filter(|file| file.language.is_ts_family())
        {
            for flow in collect_ts_token_source_flows(interner, file, None) {
                self.project_ts_token_source_flow(interner, &flow, &context);
            }
        }
    }

    fn project_ts_token_source_flow(
        &mut self,
        interner: &StableKeyInterner,
        flow: &TsTokenSourceFlow,
        context: &TsDirectBindingNodeContext<'_>,
    ) {
        let Some(source_node) =
            context.function_node_for_binding(&flow.source_function_stable_key_text)
        else {
            return;
        };
        let Some(callsite_node) = context.callsite_node_for_binding(&flow.callsite_stable_key_text)
        else {
            return;
        };
        self.push_constraint(
            interner,
            ConstraintKind::CopyEdge {
                dst: callsite_node,
                src: source_node,
            },
            &flow.stable_key_text,
        );
    }

    fn project_ts_callable_flows(&mut self, db: &impl AnalysisHost, flows: &[TsCallableFlow]) {
        use crate::analysis_neutral::points_to::facts::PointsToPrecision;
        let interner = db.stable_key_interner();
        let functions = db
            .functions()
            .iter()
            .filter_map(|function| {
                let key = function_node_key(db, function);
                self.node_for_key(&interner, &key)
                    .map(|node| (function.id, (node, key)))
            })
            .collect::<BTreeMap<_, _>>();
        let sites = db
            .call_sites()
            .iter()
            .filter_map(|site| {
                let key = node_key_from_identity("callsite", &interner.resolve(site.stable_key));
                self.node_for_key(&interner, &key)
                    .map(|node| (site.id, (node, site)))
            })
            .collect::<BTreeMap<_, _>>();
        for flow in flows {
            let (Some(&(callsite, site)), Some((target, target_key))) =
                (sites.get(&flow.site), functions.get(&flow.target_function))
            else {
                continue;
            };
            let identity = semantic_stable_key(
                FactFamily::PointsToConstraint,
                &[
                    ("site", interner.resolve(site.stable_key).to_string()),
                    ("target", target_key.clone()),
                    ("binding", flow.binding.clone()),
                    ("model", format!("callable_shape:{:?}", flow.kind)),
                ],
            )
            .into_string();
            self.inner.push_constraint_with_precision(
                &interner,
                ConstraintKind::CopyEdge {
                    dst: callsite,
                    src: *target,
                },
                &identity,
                PointsToPrecision::Heuristic,
            );
            self.inner.push_constraint_with_precision(
                &interner,
                ConstraintKind::CallConstraint { callsite },
                &identity,
                PointsToPrecision::Heuristic,
            );
        }
    }

    // -- TS object-model constraints ---------------------------------------

    fn project_ts_object_model(
        &mut self,
        db: &impl AnalysisHost,
        object_model: &TsObjectModelOutput,
    ) {
        let interner_handle = db.stable_key_interner();
        let interner = &interner_handle;
        let context = TsObjectModelNodeContext::new(interner, db, self, object_model);

        for allocation in &object_model.allocations {
            if !is_lowerable_object_model_status(&allocation.status) {
                continue;
            }
            let allocation_key = interner.resolve(allocation.stable_key);
            let Some(dst) = context.allocation_place_node(interner, self, &allocation_key) else {
                continue;
            };
            let Some(object) = context.object_node(interner, self, &allocation_key) else {
                continue;
            };
            self.push_constraint(
                interner,
                ConstraintKind::Alloc { dst, object },
                &allocation_key,
            );
        }

        for write in &object_model.property_writes {
            if !is_lowerable_object_model_status(&write.status) {
                continue;
            }
            let base_object_key = interner.resolve(write.base_object_stable_key);
            let Some(base) = context.object_node(interner, self, &base_object_key) else {
                continue;
            };
            let Some(src) = context.property_write_source_node(interner, self, write) else {
                continue;
            };
            self.push_constraint(
                interner,
                ConstraintKind::FieldStore {
                    base,
                    field: property_field_label(&write.property_key),
                    src,
                },
                &interner.resolve(write.stable_key),
            );
        }

        for read in &object_model.property_reads {
            if !is_lowerable_object_model_status(&read.status) {
                continue;
            }
            let base_object_key = interner.resolve(read.base_object_stable_key);
            let Some(base) = context.object_node(interner, self, &base_object_key) else {
                continue;
            };
            let Some(dst) = context.property_read_destination_node(interner, self, read) else {
                continue;
            };
            self.push_constraint(
                interner,
                ConstraintKind::FieldLoad {
                    dst,
                    base,
                    field: property_field_label(&read.property_key),
                },
                &interner.resolve(read.stable_key),
            );
            if let Some(callsite) = context.callsite_node(
                read.callsite_stable_key
                    .map(|key| interner.resolve(key))
                    .as_deref(),
            ) {
                self.push_constraint(
                    interner,
                    ConstraintKind::CallConstraint { callsite },
                    &interner.resolve(read.stable_key),
                );
                // The loaded property value is the callee. Keep that flow in
                // the graph explicitly; constraint identities include their
                // kind, so a FieldLoad cannot be joined to a CallConstraint by
                // comparing their complete stable keys.
                self.push_constraint(
                    interner,
                    ConstraintKind::CopyEdge {
                        dst: callsite,
                        src: dst,
                    },
                    &interner.resolve(read.stable_key),
                );
            }
        }

        for binding in &object_model.receiver_bindings {
            if !is_lowerable_object_model_status(&binding.status) {
                continue;
            }
            if let (Some(dst), Some(src)) = (
                context.receiver_destination_node(interner, self, binding),
                context.receiver_source_node(interner, self, binding),
            ) {
                self.push_constraint(
                    interner,
                    ConstraintKind::CopyEdge { dst, src },
                    &interner.resolve(binding.stable_key),
                );
            }
            if let Some(callsite) = context.callsite_node(
                binding
                    .callsite_stable_key
                    .map(|key| interner.resolve(key))
                    .as_deref(),
            ) {
                self.push_constraint(
                    interner,
                    ConstraintKind::CallConstraint { callsite },
                    &interner.resolve(binding.stable_key),
                );
            }
        }

        for link in &object_model.prototype_links {
            if !is_lowerable_object_model_status(&link.status) {
                continue;
            }
            let object_key = interner.resolve(link.object_stable_key);
            let Some(base) = context.object_node(interner, self, &object_key) else {
                continue;
            };
            let prototype_key = interner.resolve(link.prototype_stable_key);
            let Some(src) = context.object_node(interner, self, &prototype_key) else {
                continue;
            };
            self.push_constraint(
                interner,
                ConstraintKind::FieldStore {
                    base,
                    field: link
                        .property_key
                        .as_ref()
                        .map(property_field_label)
                        .unwrap_or_else(|| "$prototype".to_string()),
                    src,
                },
                &interner.resolve(link.stable_key),
            );
        }
    }

    // -- member-of edges ----------------------------------------------------

    fn project_member_of_edges(&mut self, db: &impl AnalysisHost) {
        self.inner.project_member_of_edges(db);
    }

    fn project_value_constraints(&mut self, db: &impl AnalysisHost) {
        self.inner.project_value_constraints(db);
    }

    fn into_output(self) -> SemanticGraphOutput {
        self.inner.into_output()
    }
}

fn direct_binding_emits_copy_edge(kind: TsDirectBindingKind) -> bool {
    matches!(
        kind,
        TsDirectBindingKind::LocalAlias
            | TsDirectBindingKind::ImportedNamed
            | TsDirectBindingKind::ImportedDefault
            | TsDirectBindingKind::ImportedNamespaceMember
            | TsDirectBindingKind::ReExportedAlias
            | TsDirectBindingKind::CommonJsRequireMember
    )
}

struct TsDirectBindingNodeContext<'a> {
    file_by_relative_path: BTreeMap<&'a str, FileId>,
    callsite_node_by_location: BTreeMap<(FileId, u32, u32), SemanticNodeId>,
    function_nodes_by_location: BTreeMap<(FileId, u32, u32), Vec<(String, SemanticNodeId)>>,
}

impl<'a> TsDirectBindingNodeContext<'a> {
    fn new(
        interner: &StableKeyInterner,
        db: &'a impl AnalysisHost,
        builder: &GraphBuilder,
    ) -> Self {
        let file_by_relative_path = db
            .files()
            .iter()
            .map(|file| (file.relative_path.as_str(), file.id))
            .collect();

        let callsite_node_by_location = db
            .call_sites()
            .iter()
            .filter_map(|site| {
                let key = node_key_from_identity("callsite", &interner.resolve(site.stable_key));
                let node = builder.node_for_key(interner, &key)?;
                Some(((site.file, site.span.start_byte, site.span.end_byte), node))
            })
            .collect();

        let mut function_nodes_by_location =
            BTreeMap::<(FileId, u32, u32), Vec<(String, SemanticNodeId)>>::new();
        for function in db.functions() {
            let key = function_node_key(db, function);
            let Some(node) = builder.node_for_key(interner, &key) else {
                continue;
            };
            function_nodes_by_location
                .entry((
                    function.file,
                    function.span.start_byte,
                    function.span.end_byte,
                ))
                .or_default()
                .push((function.name.clone(), node));
        }
        for nodes in function_nodes_by_location.values_mut() {
            nodes.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
        }

        Self {
            file_by_relative_path,
            callsite_node_by_location,
            function_nodes_by_location,
        }
    }

    fn callsite_node_for_binding(&self, callsite_stable_key: &str) -> Option<SemanticNodeId> {
        let identity = TsInventoryStableIdentity::parse(callsite_stable_key)?;
        let file = self.file_by_relative_path.get(identity.file)?;
        self.callsite_node_by_location
            .get(&(*file, identity.start, identity.end))
            .copied()
    }

    fn function_node_for_binding(&self, function_stable_key: &str) -> Option<SemanticNodeId> {
        let identity = TsInventoryStableIdentity::parse(function_stable_key)?;
        let file = self.file_by_relative_path.get(identity.file)?;
        self.function_nodes_by_location
            .get(&(*file, identity.start, identity.end))?
            .iter()
            .find_map(|(name, node)| {
                ts_inventory_display_matches_function_name(identity.display, name).then_some(*node)
            })
    }
}

fn ts_inventory_display_matches_function_name(display: Option<&str>, function_name: &str) -> bool {
    let Some(display) = display else {
        return true;
    };
    function_name == display
        || function_name
            .strip_suffix(display)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

struct TsObjectModelNodeContext<'a> {
    direct: TsDirectBindingNodeContext<'a>,
    object_token_by_key: BTreeMap<Arc<str>, ObjectTokenId>,
    place_by_stable_key: BTreeMap<Arc<str>, PlaceId>,
    synthetic_place_by_key: BTreeMap<String, PlaceId>,
}

impl<'a> TsObjectModelNodeContext<'a> {
    fn new(
        interner: &StableKeyInterner,
        db: &'a impl AnalysisHost,
        builder: &GraphBuilder,
        object_model: &TsObjectModelOutput,
    ) -> Self {
        let direct = TsDirectBindingNodeContext::new(interner, db, builder);
        let object_token_by_key = object_model
            .allocations
            .iter()
            .map(|allocation| interner.resolve(allocation.stable_key))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .enumerate()
            .map(|(index, stable_key)| (stable_key, ObjectTokenId(index as u64)))
            .collect();
        let place_by_stable_key = db
            .mir_places()
            .iter()
            .map(|place| (interner.resolve(place.stable_key), place.id))
            .collect::<BTreeMap<_, _>>();

        let mut synthetic_place_keys = BTreeSet::new();
        for allocation in &object_model.allocations {
            insert_synthetic_place_key(
                &place_by_stable_key,
                &mut synthetic_place_keys,
                allocation_result_place_identity(&interner.resolve(allocation.stable_key)),
            );
        }
        for write in &object_model.property_writes {
            insert_synthetic_place_key(
                &place_by_stable_key,
                &mut synthetic_place_keys,
                property_write_source_place_identity(interner, write),
            );
        }
        for read in &object_model.property_reads {
            let key = read
                .destination_stable_key
                .map(|key| interner.resolve(key).to_string())
                .unwrap_or_else(|| {
                    property_read_destination_place_identity(&interner.resolve(read.stable_key))
                });
            insert_synthetic_place_key(&place_by_stable_key, &mut synthetic_place_keys, key);
        }
        for binding in &object_model.receiver_bindings {
            let key = binding
                .receiver_place_stable_key
                .map(|key| interner.resolve(key).to_string())
                .unwrap_or_else(|| {
                    receiver_destination_place_identity(&interner.resolve(binding.stable_key))
                });
            insert_synthetic_place_key(&place_by_stable_key, &mut synthetic_place_keys, key);
        }

        let next_place_id = db
            .mir_places()
            .iter()
            .map(|place| place.id.0)
            .max()
            .map_or(0, |max_id| max_id + 1);
        let synthetic_place_by_key = synthetic_place_keys
            .into_iter()
            .enumerate()
            .map(|(index, key)| (key, PlaceId(next_place_id + index as u64)))
            .collect();

        Self {
            direct,
            object_token_by_key,
            place_by_stable_key,
            synthetic_place_by_key,
        }
    }

    fn object_node(
        &self,
        interner: &StableKeyInterner,
        builder: &mut GraphBuilder,
        object_stable_key: &str,
    ) -> Option<SemanticNodeId> {
        let token = *self.object_token_by_key.get(object_stable_key)?;
        Some(builder.intern_node(
            interner,
            NodeKind::AbstractObject(token),
            object_node_key(object_stable_key),
        ))
    }

    fn allocation_place_node(
        &self,
        interner: &StableKeyInterner,
        builder: &mut GraphBuilder,
        allocation_stable_key: &str,
    ) -> Option<SemanticNodeId> {
        self.place_node(
            interner,
            builder,
            &allocation_result_place_identity(allocation_stable_key),
        )
    }

    fn property_write_source_node(
        &self,
        interner: &StableKeyInterner,
        builder: &mut GraphBuilder,
        write: &TsPropertyWriteFact,
    ) -> Option<SemanticNodeId> {
        if let Some(value_object) = write
            .value_object_stable_key
            .map(|key| interner.resolve(key))
            && let Some(node) = self.object_node(interner, builder, &value_object)
        {
            return Some(node);
        }
        if let Some(value_function) = write
            .value_function_stable_key
            .map(|key| interner.resolve(key))
            && let Some(node) = self.direct.function_node_for_binding(&value_function)
        {
            return Some(node);
        }
        self.place_node(
            interner,
            builder,
            &property_write_source_place_identity(interner, write),
        )
    }

    fn property_read_destination_node(
        &self,
        interner: &StableKeyInterner,
        builder: &mut GraphBuilder,
        read: &TsPropertyReadFact,
    ) -> Option<SemanticNodeId> {
        let destination = read
            .destination_stable_key
            .map(|key| interner.resolve(key).to_string())
            .unwrap_or_else(|| {
                property_read_destination_place_identity(&interner.resolve(read.stable_key))
            });
        self.place_node(interner, builder, &destination)
    }

    fn receiver_destination_node(
        &self,
        interner: &StableKeyInterner,
        builder: &mut GraphBuilder,
        binding: &TsReceiverBindingFact,
    ) -> Option<SemanticNodeId> {
        let destination = binding
            .receiver_place_stable_key
            .map(|key| interner.resolve(key).to_string())
            .unwrap_or_else(|| {
                receiver_destination_place_identity(&interner.resolve(binding.stable_key))
            });
        self.place_node(interner, builder, &destination)
    }

    fn receiver_source_node(
        &self,
        interner: &StableKeyInterner,
        builder: &mut GraphBuilder,
        binding: &TsReceiverBindingFact,
    ) -> Option<SemanticNodeId> {
        if let Some(receiver_object) = binding
            .receiver_object_stable_key
            .map(|key| interner.resolve(key))
        {
            return self.object_node(interner, builder, &receiver_object);
        }
        binding
            .receiver_place_stable_key
            .map(|key| interner.resolve(key))
            .and_then(|place| self.place_node(interner, builder, &place))
    }

    fn callsite_node(&self, callsite_stable_key: Option<&str>) -> Option<SemanticNodeId> {
        self.direct.callsite_node_for_binding(callsite_stable_key?)
    }

    fn place_node(
        &self,
        interner: &StableKeyInterner,
        builder: &mut GraphBuilder,
        place_stable_key: &str,
    ) -> Option<SemanticNodeId> {
        if let Some(place) = self.place_by_stable_key.get(place_stable_key) {
            return Some(builder.intern_node(
                interner,
                NodeKind::Place(*place),
                place_node_key(place_stable_key),
            ));
        }
        let place = *self.synthetic_place_by_key.get(place_stable_key)?;
        Some(builder.intern_node(
            interner,
            NodeKind::Place(place),
            place_node_key(place_stable_key),
        ))
    }
}

fn insert_synthetic_place_key(
    place_by_stable_key: &BTreeMap<Arc<str>, PlaceId>,
    synthetic_place_keys: &mut BTreeSet<String>,
    key: String,
) {
    if !place_by_stable_key.contains_key(key.as_str()) {
        synthetic_place_keys.insert(key);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TsInventoryStableIdentity<'a> {
    file: &'a str,
    start: u32,
    end: u32,
    display: Option<&'a str>,
}

impl<'a> TsInventoryStableIdentity<'a> {
    fn parse(stable_key: &'a str) -> Option<Self> {
        let mut cursor = parse_length_prefixed_token(stable_key, 0)?.1;
        let mut file = None;
        let mut start = None;
        let mut end = None;
        let mut display = None;

        while cursor < stable_key.len() {
            if stable_key.as_bytes().get(cursor) != Some(&b'|') {
                return None;
            }
            cursor += 1;
            let (label, next) = parse_length_prefixed_token(stable_key, cursor)?;
            cursor = next;
            if stable_key.as_bytes().get(cursor) != Some(&b'=') {
                return None;
            }
            cursor += 1;
            let (value, next) = parse_length_prefixed_token(stable_key, cursor)?;
            cursor = next;

            match label {
                "file" => file = Some(value),
                "start" => start = value.parse::<u32>().ok(),
                "end" => end = value.parse::<u32>().ok(),
                "display" => display = Some(value),
                _ => {}
            }
        }

        Some(Self {
            file: file?,
            start: start?,
            end: end?,
            display,
        })
    }
}

fn parse_length_prefixed_token(input: &str, start: usize) -> Option<(&str, usize)> {
    let colon = input[start..].find(':')? + start;
    let len = input[start..colon].parse::<usize>().ok()?;
    let value_start = colon + 1;
    let value_end = value_start.checked_add(len)?;
    let value = input.get(value_start..value_end)?;
    Some((value, value_end))
}

// ---------------------------------------------------------------------------
// Stable-key composition helpers composed from referenced identities
// ---------------------------------------------------------------------------

fn object_node_key(object_stable_key: &str) -> String {
    semantic_stable_key(
        FactFamily::AllocationToken,
        &[
            ("node_kind", "abstract_object".to_string()),
            ("identity", object_stable_key.to_string()),
        ],
    )
    .into_string()
}

fn allocation_result_place_identity(allocation_stable_key: &str) -> String {
    object_model_place_identity("allocation_result", allocation_stable_key)
}

fn property_write_source_place_identity(
    interner: &StableKeyInterner,
    write: &TsPropertyWriteFact,
) -> String {
    let source_identity = write
        .value_function_stable_key
        .or(write.value_object_stable_key)
        .unwrap_or(write.stable_key);
    object_model_place_identity("property_write_source", &interner.resolve(source_identity))
}

fn property_read_destination_place_identity(read_stable_key: &str) -> String {
    object_model_place_identity("property_read_destination", read_stable_key)
}

fn receiver_destination_place_identity(receiver_stable_key: &str) -> String {
    object_model_place_identity("receiver_destination", receiver_stable_key)
}

fn object_model_place_identity(kind: &str, identity: &str) -> String {
    semantic_stable_key(
        FactFamily::Place,
        &[
            ("place_kind", kind.to_string()),
            ("identity", identity.to_string()),
        ],
    )
    .into_string()
}

fn property_field_label(property_key: &TsPropertyKey) -> String {
    property_key.field_label()
}

fn is_lowerable_object_model_status(status: &TsObjectModelStatus) -> bool {
    matches!(status, TsObjectModelStatus::Resolved)
}

#[cfg(all(test, feature = "lang-typescript"))]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use crate::analysis_api::{
        FunctionFact, ImportFact, ModuleNode, ModuleNodeKind, PackageFact, ResolutionPrecision,
        ResolutionStatus, ResolvedImportFact,
    };
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::adaptation::budget::AdaptationModelBudget;
    use crate::analysis_neutral::adaptation::facts::{
        LoadedModelFact, ModelConfidence, ModelLanguage, RejectionReason,
    };
    use crate::analysis_neutral::adaptation::store::AdaptationModelStore;
    use crate::analysis_neutral::adaptation::validate::ValidationUniverse;
    use crate::analysis_neutral::calls::facts::{
        CallCallee, CallPrecision, CallSiteFact, CallSyntaxKind, CallTargetStatus,
    };
    use crate::analysis_neutral::calls::store::CallOutput;
    use crate::analysis_neutral::ids::{CallSiteId, MirBodyId, MirOpId};
    use crate::analysis_neutral::semantic_graph::store::SemanticGraphStore;
    use crate::internal_core::{
        FileId, FunctionId, ImportId, Language, ModuleNodeId, PackageId, Span,
    };
    use crate::ts::binding::direct::{
        TsDirectBindingModuleFile, TsDirectBindingModuleInput, resolve_direct_bindings,
        resolve_direct_bindings_with_modules,
    };
    use crate::ts::binding::facts::TsDirectBindingReason;
    use crate::ts::inventory::facts::{TsInventoryCallsiteFact, TsInventoryFunctionFact};
    use crate::ts::scope::extract::extract_ts_scope;

    fn span(file: FileId) -> Span {
        Span::new(file, 0, 10, 1, 0, 1, 10)
    }

    fn build_with_object_model(
        db: &impl AnalysisHost,
        object_model: &TsObjectModelOutput,
    ) -> SemanticGraphOutput {
        build_semantic_graph_with_inputs(
            db,
            &[],
            &AdaptationModelStore::default(),
            object_model,
            None,
            &[],
            no_additional_projection,
        )
    }

    /// Builds a tiny in-memory db with one package, one function, and one call site
    /// (the function calling something) — enough to exercise >=1 node, >=1 Call edge,
    /// and >=1 CallConstraint without running the full language pipeline.
    fn fixture_db() -> LocalAnalysisDb {
        let mut db = LocalAnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("main.go"),
            "main.go".to_string(),
            "package main\nfunc main() { run() }\n".to_string(),
        );
        db.push_package(PackageFact::new(
            PackageId::from_raw(0),
            file,
            "main".to_string(),
            span(file),
            Language::Go,
        ));
        let caller = db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file,
            "main".to_string(),
            span(file),
            Language::Go,
            false,
            false,
            1,
            vec!["run".to_string()],
        ));

        let site = CallSiteFact {
            in_throw: false,
            id: CallSiteId(0),
            language: Language::Go,
            file,
            caller,
            owner_symbol: None,
            body: MirBodyId(0),
            operation: MirOpId(0),
            span: span(file),
            kind: CallSyntaxKind::Function,
            callee: CallCallee::Identifier {
                reference: None,
                name: "run".to_string(),
            },
            receiver: None,
            arguments: Vec::new(),
            result: None,
            status: CallTargetStatus::Resolved,
            precision: CallPrecision::SetupAware,
            stable_key: db
                .stable_key_interner()
                .intern("call-site:main:run".to_string()),
        };
        db.replace_call_facts(CallOutput {
            sites: vec![site],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("call replace");

        db
    }

    fn ts_db_with_file(path: &str, source: &str) -> (LocalAnalysisDb, FileId) {
        let mut db = LocalAnalysisDb::new();
        let file = db.add_file(PathBuf::from(path), path.to_string(), source.to_string());
        (db, file)
    }

    fn push_ts_function(
        db: &mut LocalAnalysisDb,
        function: &TsInventoryFunctionFact,
    ) -> FunctionId {
        db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            function.file,
            function
                .display_name
                .clone()
                .unwrap_or_else(|| "<anonymous>".to_string()),
            function.span.clone(),
            Language::TypeScript,
            false,
            false,
            1,
            Vec::new(),
        ))
    }

    fn ts_function<'a>(
        inventory: &'a TsInventoryOutput,
        display_name: &str,
    ) -> &'a TsInventoryFunctionFact {
        inventory
            .functions
            .iter()
            .find(|function| function.display_name.as_deref() == Some(display_name))
            .unwrap_or_else(|| panic!("missing TS function {display_name}"))
    }

    fn ts_callsite<'a>(
        inventory: &'a TsInventoryOutput,
        display_name: &str,
    ) -> &'a TsInventoryCallsiteFact {
        inventory
            .callsites
            .iter()
            .find(|callsite| callsite.display_name.as_deref() == Some(display_name))
            .unwrap_or_else(|| panic!("missing TS callsite {display_name}"))
    }

    fn replace_single_ts_callsite(
        db: &mut LocalAnalysisDb,
        caller: FunctionId,
        callsite: &TsInventoryCallsiteFact,
    ) {
        db.replace_call_facts(CallOutput {
            sites: vec![CallSiteFact {
                in_throw: false,
                id: CallSiteId(0),
                language: Language::TypeScript,
                file: callsite.file,
                caller,
                owner_symbol: None,
                body: MirBodyId(0),
                operation: MirOpId(0),
                span: callsite.span.clone(),
                kind: CallSyntaxKind::Function,
                callee: CallCallee::Identifier {
                    reference: None,
                    name: callsite
                        .display_name
                        .clone()
                        .unwrap_or_else(|| "<dynamic>".to_string()),
                },
                receiver: None,
                arguments: Vec::new(),
                result: None,
                status: CallTargetStatus::Resolved,
                precision: CallPrecision::SetupAware,
                stable_key: db.stable_key_interner().intern(format!(
                    "ts-callsite:{}",
                    db.resolve_stable_key(callsite.stable_key)
                )),
            }],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("call replace");
    }

    #[test]
    fn build_emits_nodes_call_edge_and_call_constraint_with_zero_model_edges() {
        let db = fixture_db();
        let output = build_semantic_graph(&db);

        // At least one node was projected from real facts.
        assert!(!output.nodes.is_empty(), "expected projected nodes");
        // A function node and a callsite node are present.
        assert!(
            output
                .nodes
                .iter()
                .any(|n| matches!(n.kind, NodeKind::Function(_))),
            "expected a function node"
        );
        assert!(
            output
                .nodes
                .iter()
                .any(|n| matches!(n.kind, NodeKind::Callsite(_))),
            "expected a callsite node"
        );

        // >=1 Call edge and >=1 CallConstraint from the call site.
        let call_edges = output
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Call)
            .count();
        assert!(call_edges >= 1, "expected >=1 Call edge, got {call_edges}");

        let call_constraints = output
            .constraints
            .iter()
            .filter(|c| matches!(c.kind, ConstraintKind::CallConstraint { .. }))
            .count();
        assert!(
            call_constraints >= 1,
            "expected >=1 CallConstraint, got {call_constraints}"
        );

        // Exactly zero ModelEdge constraints preserves honest emptiness.
        let model_edges = output
            .constraints
            .iter()
            .filter(|c| matches!(c.kind, ConstraintKind::ModelEdge { .. }))
            .count();
        assert_eq!(model_edges, 0, "ModelEdge must emit zero rows");

        // No precision label is the exact ceiling — graph rows are conservative.
        assert!(
            output
                .nodes
                .iter()
                .all(|n| n.precision == SemanticPrecision::Conservative)
        );

        // The projected graph passes referential validation end-to-end: every edge
        // endpoint and every constraint node reference resolves to a stored node.
        let store = SemanticGraphStore::from_output(output, &db.stable_key_interner())
            .expect("graph validates");
        assert!(!store.nodes().is_empty());
    }

    #[test]
    fn semantic_graph_model_edge_lowers_only_accepted_model_facts() {
        let db = fixture_db();
        let base = build_semantic_graph(&db);
        let source = node_key(&db, &base, |kind| matches!(kind, NodeKind::Callsite(_)));
        let target = node_key(&db, &base, |kind| matches!(kind, NodeKind::Function(_)));
        let store = model_store(
            vec![model_fact(&source, &target)],
            std::slice::from_ref(&source),
            std::slice::from_ref(&target),
        );

        let output =
            build_semantic_graph_with_ts_direct_bindings_and_adaptation_models(&db, &[], &store);
        let model_edges = output
            .constraints
            .iter()
            .filter_map(|constraint| match &constraint.kind {
                ConstraintKind::ModelEdge {
                    source,
                    target,
                    language,
                    scope,
                    confidence,
                    evidence,
                } => Some((*source, *target, language, scope, confidence, evidence)),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(model_edges.len(), 1);
        let (_, _, language, scope, confidence, evidence) = model_edges[0];
        assert_eq!(language, "typescript");
        assert_eq!(scope, "src/app.ts");
        assert_eq!(confidence, "heuristic");
        assert_eq!(evidence, &vec!["src/app.ts:10".to_string()]);

        let graph_store = SemanticGraphStore::from_output(output, &db.stable_key_interner())
            .expect("graph validates");
        assert_eq!(
            graph_store
                .constraints()
                .iter()
                .filter(|constraint| matches!(constraint.kind, ConstraintKind::ModelEdge { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn adaptation_model_rejections_do_not_lower_to_model_edges() {
        let db = fixture_db();
        let base = build_semantic_graph(&db);
        let source = node_key(&db, &base, |kind| matches!(kind, NodeKind::Callsite(_)));
        let target = node_key(&db, &base, |kind| matches!(kind, NodeKind::Function(_)));
        let rejected_target = model_store(
            vec![model_fact(&source, "node|function|missing")],
            std::slice::from_ref(&source),
            std::slice::from_ref(&target),
        );
        let rejected_pattern = model_store(
            vec![model_fact(&source, "function::*")],
            std::slice::from_ref(&source),
            std::slice::from_ref(&target),
        );
        let rejected_oracle = model_store(
            vec![model_fact(&source, "expected:benchmark-answer")],
            std::slice::from_ref(&source),
            std::slice::from_ref(&target),
        );

        assert_eq!(
            rejected_target.rejected()[0].reason,
            RejectionReason::NonResolvingTarget
        );
        assert_eq!(
            rejected_pattern.rejected()[0].reason,
            RejectionReason::BroadTargetPattern
        );
        assert_eq!(
            rejected_oracle.rejected()[0].reason,
            RejectionReason::OracleShapedTarget
        );

        for store in [&rejected_target, &rejected_pattern, &rejected_oracle] {
            let output =
                build_semantic_graph_with_ts_direct_bindings_and_adaptation_models(&db, &[], store);
            assert!(
                output
                    .constraints
                    .iter()
                    .all(|constraint| !matches!(constraint.kind, ConstraintKind::ModelEdge { .. }))
            );
        }

        // Control: the same source with the resolving target is accepted.
        let accepted = model_store(vec![model_fact(&source, &target)], &[source], &[target]);
        assert_eq!(accepted.accepted().len(), 1);
    }

    fn node_key(
        db: &impl AnalysisHost,
        output: &SemanticGraphOutput,
        predicate: impl Fn(&NodeKind) -> bool,
    ) -> String {
        db.resolve_stable_key(
            output
                .nodes
                .iter()
                .find(|node| predicate(&node.kind))
                .expect("node exists")
                .stable_key,
        )
        .to_string()
    }

    fn model_store(
        facts: Vec<LoadedModelFact>,
        sources: &[String],
        targets: &[String],
    ) -> AdaptationModelStore {
        let interner = crate::analysis_neutral::LocalAnalysisDb::new().stable_key_interner();
        let universe = ValidationUniverse::new(sources.iter().cloned(), targets.iter().cloned())
            .with_oracle_labels(["expected:benchmark-answer"]);
        AdaptationModelStore::build(
            &interner,
            facts,
            &universe,
            AdaptationModelBudget::default(),
        )
    }

    fn model_fact(source: &str, target: &str) -> LoadedModelFact {
        LoadedModelFact {
            model_path: ".polint/models/framework.toml".to_string(),
            source_pattern: source.to_string(),
            target_pattern: target.to_string(),
            confidence: ModelConfidence::Heuristic,
            language: ModelLanguage::TypeScript,
            scope: "src/app.ts".to_string(),
            evidence: vec!["src/app.ts:10".to_string()],
            stable_key: crate::internal_core::stable_key_for_test(&format!(
                "model:{source}->{target}"
            )),
        }
    }

    #[test]
    fn build_is_read_only_and_deterministic() {
        let db = fixture_db();
        // Building twice over the same db yields byte-identical normalized output —
        // the projection mutates nothing and is order-independent.
        let interner = db.stable_key_interner();
        let a = build_semantic_graph(&db).normalized(&interner);
        let b = build_semantic_graph(&db).normalized(&interner);
        assert_eq!(a.constraints, b.constraints);
        assert_eq!(a.nodes, b.nodes);

        // Building is read-only: the database's call sites and functions remain
        // unchanged.
        let before_sites = db.call_sites().len();
        let before_funcs = db.functions().len();
        let _ = build_semantic_graph(&db);
        assert_eq!(db.call_sites().len(), before_sites);
        assert_eq!(db.functions().len(), before_funcs);
    }

    mod ts_direct_bindings {
        use super::*;

        #[test]
        fn projects_copy_edge_and_call_constraint_for_local_alias() {
            let (mut db, file) = ts_db_with_file(
                "src/app.ts",
                r#"
function f() {}
const alias = f;
function run() {
  alias();
}
"#,
            );
            let interner = db.stable_key_interner();
            let inventory = extract_ts_inventory(&interner, db.file(file).expect("file"));
            let scope = extract_ts_scope(&interner, db.file(file).expect("file"));
            let target = ts_function(&inventory, "f").clone();
            let caller = ts_function(&inventory, "run").clone();
            let callsite = ts_callsite(&inventory, "alias").clone();
            push_ts_function(&mut db, &target);
            let caller = push_ts_function(&mut db, &caller);
            replace_single_ts_callsite(&mut db, caller, &callsite);

            let bindings = resolve_direct_bindings(&interner, &inventory, &scope);
            let output = build_semantic_graph_with_ts_direct_bindings(&db, &bindings.bindings);
            assert_ts_direct_projection(&db, output);
        }

        #[test]
        fn projects_copy_edge_and_call_constraint_for_imported_alias() {
            let mut db = LocalAnalysisDb::new();
            let app_file = db.add_file(
                PathBuf::from("src/app.ts"),
                "src/app.ts".to_string(),
                r#"
import { f as g } from "./m";
function run() {
  g();
}
"#
                .to_string(),
            );
            let module_file = db.add_file(
                PathBuf::from("src/m.ts"),
                "src/m.ts".to_string(),
                "export function f() {}".to_string(),
            );
            let interner = db.stable_key_interner();
            let app_inventory = extract_ts_inventory(&interner, db.file(app_file).expect("app"));
            let app_scope = extract_ts_scope(&interner, db.file(app_file).expect("app"));
            let module_inventory =
                extract_ts_inventory(&interner, db.file(module_file).expect("module"));
            let module_scope = extract_ts_scope(&interner, db.file(module_file).expect("module"));

            let target = ts_function(&module_inventory, "f").clone();
            let caller = ts_function(&app_inventory, "run").clone();
            let callsite = ts_callsite(&app_inventory, "g").clone();
            push_ts_function(&mut db, &target);
            let caller = push_ts_function(&mut db, &caller);
            replace_single_ts_callsite(&mut db, caller, &callsite);
            let import = db.push_import(ImportFact::new(
                ImportId::from_raw(0),
                app_file,
                None,
                "./m".to_string(),
                Span::point(app_file, 1, 0),
                Language::TypeScript,
            ));
            db.replace_module_graph_facts(
                vec![ResolvedImportFact::new(
                    crate::internal_core::ResolvedImportId::from_raw(0),
                    import,
                    app_file,
                    Some(ModuleNodeId::from_raw(1)),
                    ResolutionStatus::Resolved,
                    ResolutionPrecision::ExactFile,
                    None,
                )],
                vec![
                    ModuleNode::new(
                        ModuleNodeId::from_raw(0),
                        ModuleNodeKind::File,
                        "src/app.ts".to_string(),
                        Some(app_file),
                        None,
                        Some(Language::TypeScript),
                    ),
                    ModuleNode::new(
                        ModuleNodeId::from_raw(1),
                        ModuleNodeKind::File,
                        "src/m.ts".to_string(),
                        Some(module_file),
                        None,
                        Some(Language::TypeScript),
                    ),
                ],
                Vec::new(),
            );
            let module_files = vec![TsDirectBindingModuleFile {
                module_node: ModuleNodeId::from_raw(1),
                inventory: &module_inventory,
                scope: &module_scope,
            }];
            let module_input = TsDirectBindingModuleInput {
                imports: db.imports(),
                resolved_imports: db.resolved_imports(),
                module_nodes: db.module_nodes(),
                module_files: &module_files,
            };

            let bindings = resolve_direct_bindings_with_modules(
                &interner,
                &app_inventory,
                &app_scope,
                &module_input,
            );
            let output = build_semantic_graph_with_ts_direct_bindings(&db, &bindings.bindings);
            assert_ts_direct_projection(&db, output);
        }

        #[test]
        fn semantic_graph_only_projection_does_not_add_call_target_facts() {
            let (mut db, file) = ts_db_with_file(
                "src/app.ts",
                r#"
function f() {}
const alias = f;
function run() {
  alias();
}
"#,
            );
            let interner = db.stable_key_interner();
            let inventory = extract_ts_inventory(&interner, db.file(file).expect("file"));
            let scope = extract_ts_scope(&interner, db.file(file).expect("file"));
            let target = ts_function(&inventory, "f").clone();
            let caller = ts_function(&inventory, "run").clone();
            let callsite = ts_callsite(&inventory, "alias").clone();
            push_ts_function(&mut db, &target);
            let caller = push_ts_function(&mut db, &caller);
            replace_single_ts_callsite(&mut db, caller, &callsite);

            let bindings = resolve_direct_bindings(&interner, &inventory, &scope);
            let before_targets = db.call_targets().len();
            let output = build_semantic_graph_with_ts_direct_bindings(&db, &bindings.bindings);

            assert_ts_direct_projection(&db, output);
            assert_eq!(db.call_targets().len(), before_targets);
        }

        #[test]
        fn source_documents_semantic_graph_only_projection() {
            let source = include_str!("semantic_graph_build.rs");

            assert!(source.contains("semantic-graph-only projection"));
            assert!(source.contains("do not create CallTargetFact rows"));
        }

        #[test]
        fn unresolved_no_solver_boundary_rows_emit_zero_target_constraints() {
            let db = LocalAnalysisDb::new();
            let interner = db.stable_key_interner();
            let rows = [
                TsDirectBindingReason::TokenFlowRequired,
                TsDirectBindingReason::PropertyFlowRequired,
                TsDirectBindingReason::PrototypeModelRequired,
                TsDirectBindingReason::ThisModelRequired,
            ]
            .into_iter()
            .map(|reason| unresolved_binding(&interner, reason))
            .collect::<Vec<_>>();

            let output = build_semantic_graph_with_ts_direct_bindings(&db, &rows);

            assert!(output.constraints.iter().all(|constraint| {
                !matches!(constraint.kind, ConstraintKind::CopyEdge { .. })
                    && !matches!(constraint.kind, ConstraintKind::CallConstraint { .. })
            }));
        }

        #[test]
        fn source_has_no_solver_reference_or_token_worklist() {
            let source = include_str!("semantic_graph_build.rs");

            assert!(!source.contains(concat!("analysis", "::solver")));
            assert!(!source.contains(concat!("fixed", "-point")));
            assert!(!source.contains(concat!("token", "-set")));
            assert!(!source.contains(concat!("token", " propagation")));
        }

        fn assert_ts_direct_projection(db: &impl AnalysisHost, output: SemanticGraphOutput) {
            let copy_edges = output
                .constraints
                .iter()
                .filter(|constraint| matches!(constraint.kind, ConstraintKind::CopyEdge { .. }))
                .count();
            let call_constraints = output
                .constraints
                .iter()
                .filter(|constraint| {
                    matches!(constraint.kind, ConstraintKind::CallConstraint { .. })
                })
                .count();

            assert!(copy_edges >= 1, "expected TS direct CopyEdge");
            assert!(call_constraints >= 1, "expected TS direct CallConstraint");
            SemanticGraphStore::from_output(output, &db.stable_key_interner())
                .expect("TS direct graph validates");
        }

        fn unresolved_binding(
            interner: &StableKeyInterner,
            reason: TsDirectBindingReason,
        ) -> TsDirectBindingFact {
            TsDirectBindingFact {
                id: crate::ts::ids::TsDirectBindingId(0),
                callsite: crate::ts::ids::TsInventoryCallsiteId(1),
                callsite_stable_key: interner.intern(format!("callsite:{}", reason.as_str())),
                target_function: None,
                target_function_stable_key: None,
                scope_binding: None,
                scope_binding_stable_key: None,
                resolved_import: None,
                module_node: None,
                kind: TsDirectBindingKind::LocalFunction,
                status: TsDirectBindingStatus::Unresolved,
                reason: Some(reason),
                stable_key: interner.intern(format!("ts_direct_binding:{}", reason.as_str())),
            }
        }
    }

    mod ts_object_model {
        use super::*;
        use crate::ts::object_model::facts::{
            TsObjectAllocationFact, TsObjectAllocationId, TsObjectAllocationKind,
            TsObjectModelStatus, TsPropertyKey, TsPropertyKeyKind, TsPropertyReadFact,
            TsPropertyReadId, TsPropertyWriteFact, TsPropertyWriteId, TsPrototypeLinkFact,
            TsPrototypeLinkId, TsPrototypeLinkKind, TsReceiverBindingFact, TsReceiverBindingId,
            TsReceiverBindingKind,
        };
        use crate::ts::object_model::store::TsObjectModelOutput;

        #[test]
        fn lowers_object_model_rows_to_closed_constraint_vocabulary() {
            let (db, file) = ts_db_with_file("src/app.ts", "const holder = { target: fn };\n");
            let interner = db.stable_key_interner();
            let holder = "object:holder";
            let target_object = "object:target";
            let class_object = "object:class";
            let prototype_object = "object:class:prototype";

            let object_model = TsObjectModelOutput {
                allocations: vec![
                    allocation(
                        &interner,
                        file,
                        holder,
                        TsObjectAllocationKind::ObjectLiteral,
                        10,
                    ),
                    allocation(
                        &interner,
                        file,
                        target_object,
                        TsObjectAllocationKind::FunctionObject,
                        11,
                    ),
                    allocation(
                        &interner,
                        file,
                        class_object,
                        TsObjectAllocationKind::ClassObject,
                        12,
                    ),
                    allocation(
                        &interner,
                        file,
                        prototype_object,
                        TsObjectAllocationKind::PrototypeObject,
                        13,
                    ),
                ],
                property_writes: vec![property_write(&interner, file, holder, target_object)],
                property_reads: vec![computed_property_read(&interner, file, holder)],
                receiver_bindings: vec![receiver_binding(&interner, file, holder)],
                prototype_links: vec![prototype_link(
                    &interner,
                    file,
                    class_object,
                    prototype_object,
                )],
            };

            let output = build_with_object_model(&db, &object_model);

            assert!(
                output
                    .constraints
                    .iter()
                    .any(|constraint| matches!(constraint.kind, ConstraintKind::Alloc { .. })),
                "expected allocation constraint"
            );
            assert!(output.constraints.iter().any(|constraint| {
                matches!(
                    &constraint.kind,
                    ConstraintKind::FieldStore { field, .. } if field == "exact:target"
                )
            }));
            assert!(output.constraints.iter().any(|constraint| {
                matches!(
                    &constraint.kind,
                    ConstraintKind::FieldLoad { field, .. } if field == "computed_bucket"
                )
            }));
            assert!(output.constraints.iter().any(|constraint| {
                matches!(
                    &constraint.kind,
                    ConstraintKind::FieldStore { field, .. } if field == "$prototype"
                )
            }));
            assert!(output.constraints.iter().any(|constraint| {
                matches!(constraint.kind, ConstraintKind::CopyEdge { .. })
                    && db
                        .resolve_stable_key(constraint.stable_key)
                        .contains("receiver")
            }));
            SemanticGraphStore::from_output(output, &db.stable_key_interner())
                .expect("object model graph validates");
        }

        #[test]
        fn object_backed_read_with_real_callsite_emits_call_constraint() {
            let (mut db, file) = ts_db_with_file(
                "src/app.ts",
                r#"
function run() {
  holder.target();
}
"#,
            );
            let interner = db.stable_key_interner();
            let inventory = extract_ts_inventory(&interner, db.file(file).expect("file"));
            let caller = ts_function(&inventory, "run").clone();
            let caller = push_ts_function(&mut db, &caller);
            let callsite = ts_callsite(&inventory, "holder.target").clone();
            replace_single_ts_callsite(&mut db, caller, &callsite);

            let object_model = TsObjectModelOutput {
                allocations: vec![allocation(
                    &interner,
                    file,
                    "object:holder",
                    TsObjectAllocationKind::ObjectLiteral,
                    10,
                )],
                property_writes: Vec::new(),
                property_reads: vec![TsPropertyReadFact {
                    id: TsPropertyReadId(0),
                    file,
                    span: span(file),
                    stable_key: interner.intern("read:holder.target"),
                    base_object_stable_key: interner.intern("object:holder"),
                    property_key: static_property("target"),
                    destination_stable_key: None,
                    callsite: None,
                    callsite_stable_key: Some(callsite.stable_key),
                    status: TsObjectModelStatus::resolved(),
                }],
                receiver_bindings: Vec::new(),
                prototype_links: Vec::new(),
            };

            let output = build_with_object_model(&db, &object_model);

            assert!(output.constraints.iter().any(|constraint| {
                matches!(constraint.kind, ConstraintKind::CallConstraint { .. })
                    && db
                        .resolve_stable_key(constraint.stable_key)
                        .contains("read:holder.target")
            }));
            let loaded = output
                .constraints
                .iter()
                .find_map(|constraint| match constraint.kind {
                    ConstraintKind::FieldLoad { dst, .. } => Some(dst),
                    _ => None,
                })
                .expect("the property value is loaded");
            let callsite = output
                .nodes
                .iter()
                .find_map(|node| matches!(node.kind, NodeKind::Callsite(_)).then_some(node.id))
                .expect("the callsite has a semantic node");
            assert!(
                output.constraints.iter().any(|constraint| {
                    constraint.kind
                        == ConstraintKind::CopyEdge {
                            dst: callsite,
                            src: loaded,
                        }
                }),
                "property values must flow into the callee variable"
            );
            SemanticGraphStore::from_output(output, &db.stable_key_interner())
                .expect("object callsite graph validates");
        }

        fn allocation(
            interner: &StableKeyInterner,
            file: FileId,
            stable_key: &str,
            kind: TsObjectAllocationKind,
            id: u64,
        ) -> TsObjectAllocationFact {
            TsObjectAllocationFact {
                id: TsObjectAllocationId(id),
                file,
                span: span(file),
                stable_key: interner.intern(stable_key),
                lexical_parent_key: Some(interner.intern("scope:module")),
                inventory_function: None,
                inventory_function_stable_key: None,
                inventory_callsite: None,
                inventory_callsite_stable_key: None,
                kind,
                status: TsObjectModelStatus::resolved(),
            }
        }

        fn property_write(
            interner: &StableKeyInterner,
            file: FileId,
            base_object: &str,
            value_object: &str,
        ) -> TsPropertyWriteFact {
            TsPropertyWriteFact {
                id: TsPropertyWriteId(0),
                file,
                span: span(file),
                stable_key: interner.intern("write:holder.target"),
                base_object_stable_key: interner.intern(base_object),
                property_key: static_property("target"),
                value_function: None,
                value_function_stable_key: None,
                value_object_stable_key: Some(interner.intern(value_object)),
                status: TsObjectModelStatus::resolved(),
            }
        }

        fn computed_property_read(
            interner: &StableKeyInterner,
            file: FileId,
            base_object: &str,
        ) -> TsPropertyReadFact {
            TsPropertyReadFact {
                id: TsPropertyReadId(0),
                file,
                span: span(file),
                stable_key: interner.intern("read:holder[computed]"),
                base_object_stable_key: interner.intern(base_object),
                property_key: TsPropertyKey {
                    kind: TsPropertyKeyKind::ComputedBucket,
                    value: None,
                },
                destination_stable_key: None,
                callsite: None,
                callsite_stable_key: None,
                status: TsObjectModelStatus::resolved(),
            }
        }

        fn receiver_binding(
            interner: &StableKeyInterner,
            file: FileId,
            receiver_object: &str,
        ) -> TsReceiverBindingFact {
            TsReceiverBindingFact {
                id: TsReceiverBindingId(0),
                file,
                span: span(file),
                stable_key: interner.intern("receiver:holder.target"),
                kind: TsReceiverBindingKind::MethodCall,
                callsite: None,
                callsite_stable_key: None,
                callee_function: None,
                callee_function_stable_key: None,
                receiver_object_stable_key: Some(interner.intern(receiver_object)),
                receiver_place_stable_key: None,
                lexical_parent_key: Some(interner.intern("scope:module")),
                status: TsObjectModelStatus::resolved(),
            }
        }

        fn prototype_link(
            interner: &StableKeyInterner,
            file: FileId,
            object: &str,
            prototype: &str,
        ) -> TsPrototypeLinkFact {
            TsPrototypeLinkFact {
                id: TsPrototypeLinkId(0),
                file,
                span: span(file),
                stable_key: interner.intern("prototype:class"),
                kind: TsPrototypeLinkKind::ClassPrototype,
                object_stable_key: interner.intern(object),
                prototype_stable_key: interner.intern(prototype),
                property_key: None,
                status: TsObjectModelStatus::resolved(),
            }
        }

        fn static_property(name: &str) -> TsPropertyKey {
            TsPropertyKey {
                kind: TsPropertyKeyKind::Static,
                value: Some(name.to_string()),
            }
        }
    }
}

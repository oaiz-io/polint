//! Projects the lowered value flow of each TS/JS file into the semantic graph.
//!
//! A lowered file's rows become places and constraints of their own, joined to
//! the rest of the graph through the function nodes and their parameter,
//! `this` and return places, and through the call site nodes. Where the rows
//! describe a call site's callee completely, the rows' callee replaces every
//! other source of the site's callee value; everywhere else the rows' call is
//! added beside what earlier projections said, on a call site of its own.
//!
//! The projection never makes the solve more certain than the rows: every
//! variable the rows leave open copies an open place, every value they let
//! escape is passed to an unresolved call, and a call the rows do not describe
//! passes open values to whatever it may call.

use std::collections::{BTreeMap, BTreeSet};

use crate::analysis_api::{FunctionFact, ResolutionStatus};
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::ids::{PlaceId, SemanticNodeId};
use crate::analysis_neutral::points_to::facts::PointsToPrecision;
use crate::analysis_neutral::semantic_graph::build::{function_node_key, node_key_from_identity};
use crate::analysis_neutral::semantic_graph::constraints::ConstraintKind;
use crate::analysis_neutral::semantic_graph::facts::NodeKind;
use crate::internal_core::{FileId, StableKeyInterner};
use crate::ts::constraints::described::{Described, Obj, Token, describe_program};
use crate::ts::constraints::{
    FlowCall, FlowRow, FlowSpan, FlowVar, TsValueFlows, lower_ts_source_value_flows,
};

use super::{GraphBuilder, TsFileAnalysis};

/// A function node of one file, with the key its signature places derive from.
struct FunctionNode<'a> {
    function: &'a FunctionFact,
    node: SemanticNodeId,
    key: String,
}

/// The graph nodes of one file, by location.
#[derive(Default)]
struct FileNodes<'a> {
    functions: BTreeMap<FlowSpan, Vec<FunctionNode<'a>>>,
    callsites: BTreeMap<FlowSpan, Vec<SemanticNodeId>>,
}

impl<'a> FileNodes<'a> {
    /// The function with this span, when exactly one node has it.
    fn function(&self, span: FlowSpan) -> Option<&FunctionNode<'a>> {
        match self.functions.get(&span).map(Vec::as_slice) {
            Some([function]) => Some(function),
            _ => None,
        }
    }

    /// The call site with this span, when exactly one node has it.
    fn callsite(&self, span: FlowSpan) -> Option<SemanticNodeId> {
        match self.callsites.get(&span).map(Vec::as_slice) {
            Some([node]) => Some(*node),
            _ => None,
        }
    }
}

/// Constraints the projection adds once every file is read, after the ones
/// it replaces are removed.
#[derive(Default)]
struct Pending {
    constraints: Vec<(ConstraintKind, String)>,
    /// Call sites whose callee the rows name: earlier writes of their callee
    /// value and earlier calls at them are removed.
    named: BTreeSet<SemanticNodeId>,
    /// Return places of lowered functions: the rows describe every value they
    /// return, so the earlier summary of the returns is removed.
    returns: BTreeSet<SemanticNodeId>,
}

impl GraphBuilder {
    /// Projects the value flow of every TS/JS file; see the module docs.
    pub(super) fn project_ts_value_flows(
        &mut self,
        db: &impl AnalysisHost,
        ts_analyses: Option<&[TsFileAnalysis]>,
    ) {
        let interner_handle = db.stable_key_interner();
        let interner = &interner_handle;
        let recomputed;
        let flows: Vec<(FileId, &TsValueFlows)> = match ts_analyses {
            Some(analyses) => analyses
                .iter()
                .map(|analysis| (analysis.file, &analysis.value_flows))
                .collect(),
            None => {
                recomputed = db
                    .files()
                    .iter()
                    .filter(|file| file.language.is_ts_family())
                    .map(|file| (file.id, lower_ts_source_value_flows(file)))
                    .collect::<Vec<_>>();
                recomputed
                    .iter()
                    .map(|(file, flows)| (*file, flows))
                    .collect()
            }
        };

        let mut nodes = BTreeMap::<FileId, FileNodes<'_>>::new();
        for function in db.functions() {
            if !function.language.is_ts_family() {
                continue;
            }
            let key = function_node_key(db, function);
            let Some(node) = self.node_for_key(interner, &key) else {
                continue;
            };
            nodes
                .entry(function.file)
                .or_default()
                .functions
                .entry((function.span.start_byte, function.span.end_byte))
                .or_default()
                .push(FunctionNode {
                    function,
                    node,
                    key,
                });
        }
        for &(file, _) in &flows {
            nodes.entry(file).or_default();
        }
        let mut sites = Vec::new();
        for site in db.call_sites() {
            if !site.language.is_ts_family() {
                continue;
            }
            let key = node_key_from_identity("callsite", &interner.resolve(site.stable_key));
            let Some(node) = self.node_for_key(interner, &key) else {
                continue;
            };
            let span = (site.span.start_byte, site.span.end_byte);
            nodes
                .entry(site.file)
                .or_default()
                .callsites
                .entry(span)
                .or_default()
                .push(node);
            sites.push((site.file, span, node, interner.resolve(site.stable_key)));
        }
        if nodes.is_empty() {
            return;
        }

        let file_ids = flows.iter().map(|(file, _)| *file).collect::<Vec<_>>();
        let file_paths = file_ids
            .iter()
            .map(|&file| db.path_for(file))
            .collect::<Vec<_>>();
        let indices = file_ids
            .iter()
            .enumerate()
            .map(|(index, &file)| (file, index))
            .collect::<BTreeMap<_, _>>();
        let module_files = db
            .module_nodes()
            .iter()
            .filter_map(|node| Some((node.id, node.file?)))
            .collect::<BTreeMap<_, _>>();
        let resolved = db
            .resolved_imports()
            .iter()
            .map(|import| {
                (
                    import.import,
                    import
                        .target_node
                        .filter(|_| import.status == ResolutionStatus::Resolved),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut targets = BTreeMap::<(FileId, &str), BTreeSet<Option<usize>>>::new();
        for import in db.imports() {
            let target = resolved
                .get(&import.id)
                .copied()
                .flatten()
                .and_then(|node| module_files.get(&node))
                .and_then(|file| indices.get(file))
                .copied();
            targets
                .entry((import.file, &import.path))
                .or_default()
                .insert(target);
        }
        let requests = flows
            .iter()
            .map(|(file, file_flows)| {
                file_flows
                    .requests
                    .iter()
                    .map(|request| {
                        let candidates = targets.get(&(*file, request.as_ref()))?;
                        if candidates.len() != 1 {
                            return None;
                        }
                        candidates
                            .first()
                            .copied()
                            .flatten()
                            .filter(|&target| flows[target].1.lowered)
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let flow_refs = flows.iter().map(|(_, flows)| *flows).collect::<Vec<_>>();
        let described = describe_program(
            &flow_refs,
            |file, span| nodes[&file_ids[file]].function(span).is_some(),
            |file, request| requests[file].get(request as usize).copied().flatten(),
        );
        let earlier_callees = self
            .inner
            .constraints()
            .iter()
            .filter_map(|constraint| match constraint.kind {
                ConstraintKind::CallConstraint {
                    callsite,
                    callee: Some(_),
                    ..
                } => Some(callsite),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        let mut results_by_callsite = BTreeMap::<SemanticNodeId, Vec<SemanticNodeId>>::new();
        for constraint in self.inner.constraints() {
            if let ConstraintKind::CallConstraint {
                callsite,
                result: Some(result),
                ..
            } = constraint.kind
            {
                results_by_callsite
                    .entry(callsite)
                    .or_default()
                    .push(result);
            }
        }

        let mut next_place = self.inner.next_place_id();
        let mut pending = Pending::default();
        let mut described_sites = BTreeSet::<(FileId, FlowSpan)>::new();
        let max_params = flows
            .iter()
            .flat_map(|(_, flows)| flows.functions.iter().map(|function| function.params))
            .max()
            .unwrap_or(0);
        for (file_index, &(file, flows)) in flows.iter().enumerate() {
            if !flows.lowered {
                continue;
            }
            // A file without nodes belongs to a language the plan leaves out.
            let Some(file_nodes) = nodes.get(&file) else {
                continue;
            };
            let mut projection = FileProjection {
                builder: self,
                interner,
                path: db.path_for(file),
                nodes: file_nodes,
                all_nodes: &nodes,
                file_ids: &file_ids,
                file_paths: &file_paths,
                file_index,
                requests: &requests[file_index],
                earlier_callees: &earlier_callees,
                places: BTreeMap::new(),
                next_place: &mut next_place,
                pending: &mut pending,
                rows: 0,
            };
            projection.project(flows, &described[file_index], &results_by_callsite);
            described_sites.extend(flows.calls.iter().map(|call| (file, call.site)));
        }

        // A call the rows do not describe may call a lowered function with
        // anything: it passes open values to every parameter and to `this`.
        let open = self.value_flow_place(interner, &mut next_place, "guard#open".to_string());
        for (file, span, node, site_key) in sites {
            if described_sites.contains(&(file, span)) {
                continue;
            }
            pending.constraints.push((
                ConstraintKind::CallConstraint {
                    callsite: node,
                    callee: None,
                    receiver: Some(open),
                    arguments: vec![open; usize::from(max_params)],
                    spread_from: None,
                    result: None,
                },
                format!("value_flow_guard:{site_key}"),
            ));
        }

        let Pending {
            constraints,
            named,
            returns,
        } = pending;
        self.inner
            .retain_constraints(|constraint| match &constraint.kind {
                ConstraintKind::CopyEdge { dst, .. } => {
                    !(named.contains(dst)
                        || (returns.contains(dst)
                            && constraint.precision == PointsToPrecision::SummaryProjected))
                }
                ConstraintKind::Alloc { dst, .. } | ConstraintKind::FieldLoad { dst, .. } => {
                    !named.contains(dst)
                }
                ConstraintKind::CallConstraint { callsite, .. } => !named.contains(callsite),
                ConstraintKind::FieldStore { .. }
                | ConstraintKind::ModelEdge { .. }
                | ConstraintKind::TypeConstraint { .. } => true,
            });
        for (kind, identity) in constraints {
            self.push_constraint(interner, kind, &identity);
        }
    }

    /// A place of the value flow that stands for no MIR place.
    fn value_flow_place(
        &mut self,
        interner: &StableKeyInterner,
        next_place: &mut u64,
        identity: String,
    ) -> SemanticNodeId {
        let key = node_key_from_identity("value_flow", &identity);
        if let Some(node) = self.node_for_key(interner, &key) {
            return node;
        }
        let place = PlaceId(*next_place);
        *next_place += 1;
        self.intern_node(interner, NodeKind::Place(place), key)
    }
}

/// The projection of one lowered file.
struct FileProjection<'b, 'n> {
    builder: &'b mut GraphBuilder,
    interner: &'b StableKeyInterner,
    path: String,
    nodes: &'n FileNodes<'n>,
    all_nodes: &'n BTreeMap<FileId, FileNodes<'n>>,
    file_ids: &'n [FileId],
    file_paths: &'n [String],
    file_index: usize,
    requests: &'n [Option<usize>],
    earlier_callees: &'n BTreeSet<SemanticNodeId>,
    places: BTreeMap<(FlowVar, bool), SemanticNodeId>,
    next_place: &'b mut u64,
    pending: &'b mut Pending,
    rows: usize,
}

impl FileProjection<'_, '_> {
    fn project(
        &mut self,
        flows: &TsValueFlows,
        described: &Described,
        results_by_callsite: &BTreeMap<SemanticNodeId, Vec<SemanticNodeId>>,
    ) {
        let nodes = self.nodes;
        // Prim holds no object, and is written by itself so that the solve
        // does not take it for an unwritten, open place.
        let prim = self.var(FlowVar::Prim, false);
        self.push(ConstraintKind::CopyEdge {
            dst: prim,
            src: prim,
        });
        // The sink is the argument of a call to nothing the graph knows: an
        // unresolved call, whose arguments escape.
        let sink = self.var(FlowVar::Sink, true);
        let sink_call = self.place("sink_call".to_string());
        self.push(ConstraintKind::CallConstraint {
            callsite: sink_call,
            callee: None,
            receiver: None,
            arguments: vec![sink],
            spread_from: None,
            result: None,
        });

        // A parameter no call fills, the `this` of a plain call and a
        // function that falls off its end are `undefined`.
        for function in &flows.functions {
            let Some(node) = nodes.function(function.span) else {
                continue;
            };
            let ret = self.signature_node(node, "function_return", None);
            self.pending.returns.insert(ret);
            let this = self.signature_node(node, "function_this", None);
            self.push(ConstraintKind::CopyEdge {
                dst: ret,
                src: prim,
            });
            self.push(ConstraintKind::CopyEdge {
                dst: this,
                src: prim,
            });
            for position in 0..function.params {
                let param = self.signature_node(node, "function_param", Some(position));
                self.push(ConstraintKind::CopyEdge {
                    dst: param,
                    src: prim,
                });
            }
        }

        for row in &flows.rows {
            let kind = match row {
                FlowRow::Copy { dst, src } => ConstraintKind::CopyEdge {
                    dst: self.var(*dst, true),
                    src: self.var(*src, false),
                },
                FlowRow::Alloc { dst, object } => ConstraintKind::Alloc {
                    dst: self.var(*dst, true),
                    object: self.object_place(self.file_index, (*object).into()),
                },
                FlowRow::Load { dst, base, field } => ConstraintKind::FieldLoad {
                    dst: self.var(*dst, true),
                    base: self.var(*base, false),
                    field: field.to_string(),
                },
                FlowRow::Store { base, field, src } => ConstraintKind::FieldStore {
                    base: self.var(*base, false),
                    field: field.to_string(),
                    src: self.var(*src, false),
                },
            };
            self.push(kind);
        }

        for &span in &described.prototypes {
            let Some(function) = nodes.function(span) else {
                continue;
            };
            let prototype =
                self.token_value(Token::Object(self.file_index as u32, Obj::Prototype(span)));
            self.push(ConstraintKind::FieldStore {
                base: function.node,
                field: "exact:prototype".into(),
                src: prototype,
            });
        }
        for &(row, token) in &described.lookups {
            let FlowRow::Load { dst, field, .. } = &flows.rows[row] else {
                continue;
            };
            let base = self.token_value(token);
            let dst = self.var(*dst, true);
            self.push(ConstraintKind::FieldLoad {
                dst,
                base,
                field: field.to_string(),
            });
        }

        for (index, call) in flows.calls.iter().enumerate() {
            self.call(index, call, described, results_by_callsite);
        }

        let open = self.var(FlowVar::Open, false);
        for var in &described.open {
            if matches!(
                var,
                FlowVar::Open | FlowVar::Sink | FlowVar::Prim | FlowVar::Function(_)
            ) {
                continue;
            }
            let dst = self.var(*var, true);
            if dst != sink {
                self.push(ConstraintKind::CopyEdge { dst, src: open });
            }
        }
        for span in &described.escaped_functions {
            if let Some(function) = nodes.function(*span) {
                let src = function.node;
                self.push(ConstraintKind::CopyEdge { dst: sink, src });
            }
        }
        for object in &described.escaped_objects {
            let src = self.token_value(Token::Object(self.file_index as u32, *object));
            self.push(ConstraintKind::CopyEdge { dst: sink, src });
        }
    }

    fn call(
        &mut self,
        index: usize,
        call: &FlowCall,
        described: &Described,
        results_by_callsite: &BTreeMap<SemanticNodeId, Vec<SemanticNodeId>>,
    ) {
        if let Some(model) = &call.model {
            if let Some(source) = self.nodes.callsite(call.site) {
                for token in described.model_targets.get(&index).into_iter().flatten() {
                    if let Token::Function(file, span) = token
                        && let Some(function) =
                            self.all_nodes[&self.file_ids[*file as usize]].function(*span)
                    {
                        self.push(ConstraintKind::ModelEdge {
                            source,
                            target: function.node,
                            language: "javascript".into(),
                            scope: self.path.clone(),
                            confidence: "conservative".into(),
                            evidence: vec![
                                format!("platform:{model}"),
                                format!(
                                    "platform_digest:{}",
                                    crate::ts::constraints::platform::digest(
                                        crate::ts::constraints::platform::SOURCE
                                    )
                                    .value
                                ),
                            ],
                        });
                    }
                }
            }
            // Argument and return flow uses an internal call site. Only the
            // validated model edge reaches the source call site's targets.
            let callsite = self.place(format!("platform#{}:{}#{index}", call.site.0, call.site.1));
            let callee = self.var(call.callee, false);
            let receiver = call.receiver.map(|var| self.var(var, false));
            let arguments = call
                .arguments
                .iter()
                .map(|var| self.var(*var, false))
                .collect();
            let result = self.var(call.result, true);
            self.push(ConstraintKind::CallConstraint {
                callsite,
                callee: Some(callee),
                receiver,
                arguments,
                spread_from: call.spread_from,
                result: Some(result),
            });
            return;
        }
        let callee = self.var(call.callee, false);
        let receiver = call.receiver.map(|receiver| self.var(receiver, false));
        let mut arguments = call
            .arguments
            .iter()
            .map(|argument| self.var(*argument, false))
            .collect::<Vec<_>>();
        let result = self.var(call.result, true);
        let named = described.named.get(&call.site).copied();
        if named.is_none() {
            // A non-callable object can occur beside a function without an
            // unknown token. That alternative still prevents a complete answer.
            let open = self.var(FlowVar::Open, false);
            // The open source must stay unwritten, and Prim must stay closed.
            if callee != open && call.callee != FlowVar::Prim {
                self.push(ConstraintKind::CopyEdge {
                    dst: callee,
                    src: open,
                });
            }
        }
        let real_site = self.nodes.callsite(call.site);
        let site = real_site.filter(|site| named.is_some() || !self.earlier_callees.contains(site));
        if let (Some(params), None) = (named, call.spread_from) {
            // A parameter the call leaves out is `undefined`.
            let prim = self.var(FlowVar::Prim, false);
            arguments.resize(arguments.len().max(usize::from(params)), prim);
        }
        let callsite = match site {
            Some(site) => {
                if named.is_some() {
                    self.pending.named.insert(site);
                }
                for &earlier in results_by_callsite.get(&site).into_iter().flatten() {
                    self.push(ConstraintKind::CopyEdge {
                        dst: earlier,
                        src: result,
                    });
                }
                site
            }
            None => self.place(format!("call#{}:{}#{index}", call.site.0, call.site.1)),
        };
        self.push(ConstraintKind::CallConstraint {
            callsite,
            callee: Some(callee),
            receiver,
            arguments,
            spread_from: call.spread_from,
            result: Some(result),
        });
    }

    /// The node of a variable. A variable of a function the graph has no node
    /// for is open where it is read, and escapes where it is written.
    fn var(&mut self, var: FlowVar, written: bool) -> SemanticNodeId {
        if let Some(node) = self.places.get(&(var, written)) {
            return *node;
        }
        let missing = if written {
            FlowVar::Sink
        } else {
            FlowVar::Open
        };
        let nodes = self.nodes;
        let node = match var {
            FlowVar::Namespace => self.place("namespace".into()),
            FlowVar::Module => self.place("module".into()),
            FlowVar::Exports => self.place("exports".into()),
            FlowVar::RequestNamespace(request) | FlowVar::RequestModule(request) => {
                let namespace = matches!(var, FlowVar::RequestNamespace(_));
                let dst = self.place(format!(
                    "request#{}#{request}",
                    if namespace { "namespace" } else { "module" }
                ));
                let src = match self.requests.get(request as usize).copied().flatten() {
                    Some(target) => self.builder.value_flow_place(
                        self.interner,
                        self.next_place,
                        format!(
                            "{}#{}",
                            self.file_paths[target],
                            if namespace { "namespace" } else { "module" }
                        ),
                    ),
                    None => self.var(FlowVar::Open, false),
                };
                self.push(ConstraintKind::CopyEdge { dst, src });
                dst
            }
            FlowVar::Local(symbol) => self.place(format!("local#{symbol}")),
            FlowVar::Temp(temp) => self.place(format!("temp#{temp}")),
            FlowVar::Open => self.place("open".to_string()),
            FlowVar::Sink => self.place("sink".to_string()),
            FlowVar::Prim => self.place("prim".to_string()),
            FlowVar::Function(span) => match nodes.function(span) {
                Some(function) => function.node,
                None => self.var(missing, written),
            },
            FlowVar::Param(span, position) => match nodes.function(span) {
                Some(function) => self.signature_node(function, "function_param", Some(position)),
                None => self.var(missing, written),
            },
            FlowVar::This(span) => match nodes.function(span) {
                Some(function) => self.signature_node(function, "function_this", None),
                None => self.var(missing, written),
            },
            FlowVar::Return(span) => match nodes.function(span) {
                Some(function) => self.signature_node(function, "function_return", None),
                None => self.var(missing, written),
            },
        };
        self.places.insert((var, written), node);
        node
    }

    /// A parameter, `this` or return place of a function node.
    fn signature_node(
        &mut self,
        function: &FunctionNode<'_>,
        kind: &str,
        position: Option<u8>,
    ) -> SemanticNodeId {
        let id = function.function.id;
        let (node_kind, identity) = match (kind, position) {
            ("function_param", Some(position)) => (
                NodeKind::FunctionParam(id, position),
                format!("{}#{position}", function.key),
            ),
            ("function_this", _) => (NodeKind::FunctionThis(id), function.key.clone()),
            _ => (NodeKind::FunctionReturn(id), function.key.clone()),
        };
        self.builder.intern_node(
            self.interner,
            node_kind,
            node_key_from_identity(kind, &identity),
        )
    }

    fn object_place(&mut self, file: usize, object: Obj) -> SemanticNodeId {
        let suffix = match object {
            Obj::Site(span) => format!("{}:{}", span.0, span.1),
            Obj::Prototype(span) => format!("prototype#{}:{}", span.0, span.1),
            Obj::Namespace => "namespace".into(),
            Obj::Module => "module".into(),
            Obj::Exports => "exports".into(),
        };
        self.builder.value_flow_place(
            self.interner,
            self.next_place,
            format!("{}#object#{suffix}", self.file_paths[file]),
        )
    }

    fn token_value(&mut self, token: Token) -> SemanticNodeId {
        let tmp = self.place(format!("lookup#{}", self.rows));
        match token {
            Token::Object(file, object) => {
                let object = self.object_place(file as usize, object);
                self.push(ConstraintKind::Alloc { dst: tmp, object });
            }
            Token::Function(file, span) => {
                let src = self.all_nodes[&self.file_ids[file as usize]]
                    .function(span)
                    .map(|function| function.node);
                let src = src.unwrap_or_else(|| self.var(FlowVar::Open, false));
                self.push(ConstraintKind::CopyEdge { dst: tmp, src });
            }
            Token::Unknown | Token::Prim => {}
        }
        tmp
    }

    fn place(&mut self, local: String) -> SemanticNodeId {
        let identity = format!("{}#{local}", self.path);
        self.builder
            .value_flow_place(self.interner, self.next_place, identity)
    }

    fn push(&mut self, kind: ConstraintKind) {
        let identity = format!("value_flow:{}#row#{}", self.path, self.rows);
        self.rows += 1;
        self.pending.constraints.push((kind, identity));
    }
}

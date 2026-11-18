//! Exports the TS/JS points-to inputs of one package, with the answers of
//! their solve, as a package graph.
//!
//! The package is analysed with its own directory as the root, so the paths
//! and node texts in the graph are relative to the package.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::{TsPointsToInputs, site_answers, solve_ts_points_to};
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::ids::SemanticNodeId;
use crate::analysis_neutral::package_graph::{
    Boundary, BudgetEntry, CallsiteEntry, CallsiteIdentity, ConstraintEntry, FunctionEntry,
    FunctionIdentity, PackageGraph, PackageIdentity, SignatureEntry, SiteEntry,
};
use crate::analysis_neutral::semantic_graph::constraints::ConstraintKind;
use crate::analysis_neutral::semantic_graph::facts::NodeKind;
use crate::analysis_neutral::solver::budget::SolverBudget;

/// The package graph of the package analysed into `db`, rooted at `root`.
/// `integrity` is the registry integrity value of the package tarball, when
/// the caller knows it.
pub(crate) fn export(
    db: &impl AnalysisHost,
    root: &Path,
    integrity: Option<&str>,
) -> anyhow::Result<PackageGraph> {
    let interner = db.stable_key_interner();
    let inputs = TsPointsToInputs::from_db(db);
    let result = solve_ts_points_to(&interner, &inputs, &SolverBudget::default());

    // A type constraint names a type fact by its run-local id, and the solve
    // does not read it, so the graph leaves it out.
    let constraints = inputs
        .constraints
        .iter()
        .filter(|constraint| !matches!(constraint.kind, ConstraintKind::TypeConstraint { .. }))
        .collect::<Vec<_>>();
    let signatures = signature_places(db);

    let mut referenced = BTreeSet::new();
    referenced.extend(constraints.iter().flat_map(|c| c.kind.referenced_nodes()));
    referenced.extend(inputs.callable_objects.values().map(|(node, _)| *node));
    referenced.extend(
        inputs
            .callsites
            .iter()
            .flat_map(|site| [site.callsite_node, site.caller_node]),
    );
    referenced.extend(signatures.values().flatten().map(|(_, node)| *node));

    let text_of = db
        .semantic_nodes()
        .iter()
        .map(|node| (node.id, node.stable_key))
        .collect::<BTreeMap<_, _>>();
    let mut texts = referenced
        .iter()
        .map(|node| {
            let key = text_of
                .get(node)
                .ok_or_else(|| anyhow::anyhow!("constraint names an unknown node {node:?}"))?;
            Ok((interner.resolve(*key).to_string(), *node))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    texts.sort();
    if let Some(pair) = texts.windows(2).find(|pair| pair[0].0 == pair[1].0) {
        anyhow::bail!("two nodes share the text {:?}", pair[0].0);
    }
    let number = texts
        .iter()
        .enumerate()
        .map(|(index, (_, node))| (*node, u32::try_from(index).expect("node count fits u32")))
        .collect::<BTreeMap<_, _>>();
    let number_of = |node: SemanticNodeId| number[&node];

    let files = db
        .files()
        .iter()
        .map(|file| (file.id, file))
        .collect::<BTreeMap<_, _>>();
    let function_facts = db
        .functions()
        .iter()
        .map(|function| (function.id, function))
        .collect::<BTreeMap<_, _>>();
    let callsite_facts = db
        .call_sites()
        .iter()
        .map(|site| (site.id, site))
        .collect::<BTreeMap<_, _>>();
    let kind_of = db
        .semantic_nodes()
        .iter()
        .map(|node| (node.id, node.kind.clone()))
        .collect::<BTreeMap<_, _>>();

    let mut function_identity = BTreeMap::new();
    let mut functions = Vec::new();
    for (node, _) in inputs.callable_objects.values() {
        let Some(NodeKind::Function(id)) = kind_of.get(node) else {
            continue;
        };
        let (Some(function), Some(file)) = (
            function_facts.get(id),
            function_facts.get(id).and_then(|f| files.get(&f.file)),
        ) else {
            continue;
        };
        let identity = FunctionIdentity::new(
            &file.relative_path,
            function.span.start_byte,
            function.span.end_byte,
            &file.source,
        );
        function_identity.insert(*node, identity.clone());
        functions.push(FunctionEntry {
            node: number_of(*node),
            name: function.name.clone(),
            identity,
        });
    }
    functions.sort_by_key(|function| function.node);

    let mut callsites = inputs
        .callsites
        .iter()
        .filter_map(|site| {
            let Some(NodeKind::Callsite(id)) = kind_of.get(&site.callsite_node) else {
                return None;
            };
            let fact = callsite_facts.get(id)?;
            let caller = function_identity.get(&site.caller_node)?;
            Some(CallsiteEntry {
                node: number_of(site.callsite_node),
                caller: number_of(site.caller_node),
                identity: CallsiteIdentity::within(
                    caller,
                    fact.span.start_byte,
                    fact.span.end_byte,
                ),
                precision: site.precision,
                constraint: interner.resolve(site.constraint_stable_key).to_string(),
            })
        })
        .collect::<Vec<_>>();
    callsites.sort_by_key(|site| site.node);

    let mut signature_entries = signatures
        .iter()
        .map(|(function, places)| {
            let mut entry = SignatureEntry {
                function: number_of(*function),
                ..SignatureEntry::default()
            };
            for (place, node) in places {
                let node = number_of(*node);
                match place {
                    Place::Param(position) => entry.params.push((*position, node)),
                    Place::Rest => entry.rest = Some(node),
                    Place::This => entry.this = Some(node),
                    Place::Return => entry.ret = Some(node),
                }
            }
            entry.params.sort_unstable();
            entry
        })
        .collect::<Vec<_>>();
    signature_entries.sort_by_key(|entry| entry.function);

    let mut constraint_entries = constraints
        .iter()
        .map(|constraint| {
            let mut kind = constraint.kind.clone();
            kind.remap_nodes(|node| SemanticNodeId(u64::from(number_of(node))));
            ConstraintEntry {
                kind,
                status: constraint.status,
                precision: constraint.precision,
                key: interner.resolve(constraint.stable_key).to_string(),
            }
        })
        .collect::<Vec<_>>();
    constraint_entries.sort();
    constraint_entries.dedup();

    let mut sites = site_answers(&inputs, &result)
        .into_iter()
        .map(|answer| {
            let mut targets = answer
                .targets
                .iter()
                .map(|target| number_of(*target))
                .collect::<Vec<_>>();
            targets.sort_unstable();
            SiteEntry {
                callsite: number_of(answer.callsite),
                status: answer.status,
                claim: answer.claim,
                targets,
            }
        })
        .collect::<Vec<_>>();
    sites.sort_by_key(|site| site.callsite);

    let identity = PackageIdentity::read(
        root,
        db.files()
            .iter()
            .map(|file| (file.relative_path.as_str(), file.content_hash.as_str())),
        integrity,
        crate::ts::module_graph::EXPORT_CONDITIONS,
        &crate::analysis_neutral::adaptation::platform::digest_evidence(),
    );
    Ok(PackageGraph {
        identity,
        nodes: texts.into_iter().map(|(text, _)| text).collect(),
        functions,
        callsites,
        signatures: signature_entries,
        constraints: constraint_entries,
        sites,
        boundary: Boundary::default(),
        budget: BudgetEntry {
            status: result.points_to.budget_status,
            events: result.points_to.budget_reasons.iter().cloned().collect(),
        },
    })
}

enum Place {
    Param(usize),
    Rest,
    This,
    Return,
}

/// The parameter, rest, `this` and return places of each function node.
fn signature_places(
    db: &impl AnalysisHost,
) -> BTreeMap<SemanticNodeId, Vec<(Place, SemanticNodeId)>> {
    let function_node = db
        .semantic_nodes()
        .iter()
        .filter_map(|node| match node.kind {
            NodeKind::Function(function) => Some((function, node.id)),
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    let mut places = BTreeMap::<_, Vec<_>>::new();
    for node in db.semantic_nodes() {
        let (function, place) = match node.kind {
            NodeKind::FunctionParam(function, position) => {
                (function, Place::Param(usize::from(position)))
            }
            NodeKind::FunctionRest(function) => (function, Place::Rest),
            NodeKind::FunctionThis(function) => (function, Place::This),
            NodeKind::FunctionReturn(function) => (function, Place::Return),
            _ => continue,
        };
        if let Some(function) = function_node.get(&function) {
            places.entry(*function).or_default().push((place, node.id));
        }
    }
    places
}

#[cfg(test)]
mod tests;

//! Stitches the package graphs of a program into the answers of the whole
//! program.
//!
//! The steps:
//!
//! 1. Refuse a graph whose solve its budget cut short: it is not a fixed point.
//! 2. Name every node by its package and its text, so the graphs share no node.
//! 3. Link each import the tree resolves to the module value of the file it
//!    names: beside the open value the import holds alone, it copies that
//!    file's module place. The open value stays, because each package was
//!    described with the other as outside code, as the whole program is
//!    lowered too. An import the tree does not resolve is reported.
//! 4. Widen each value-flow guard to the widest function of the union.
//! 5. Solve the union from nothing and answer every call site again: every
//!    claim is derived from the union, so no claim of a graph alone carries
//!    over.

use std::collections::{BTreeMap, BTreeSet};

use super::super::{TsPointsToCallsite, TsPointsToInputs, object_for_node, var_for_node};
use super::super::{invocations, site_answers, solve_ts_points_to};
use crate::analysis_neutral::ids::{SemanticConstraintId, SemanticNodeId};
use crate::analysis_neutral::package_graph::{
    ConstraintEntry, ImportEntry, ModuleValue, PackageGraph, SiteEntry,
};
use crate::analysis_neutral::points_to::solver::{PointsToSignature, SignaturePlace};
use crate::analysis_neutral::semantic_graph::constraints::{ConstraintFact, ConstraintKind};
use crate::analysis_neutral::solver::budget::SolverBudget;
use crate::internal_core::StableKeyInterner;

/// The key prefix of the calls the value flow does not describe.
const GUARD_KEY: &str = "value_flow_guard:";

/// Where the imports of a program lead: for each package, importing file and
/// specifier, the package and the file inside it that the import names.
#[derive(Debug, Clone, Default)]
pub(crate) struct Tree {
    pub(crate) links: BTreeMap<(String, String, String), (String, String)>,
}

#[cfg(feature = "lang-typescript")]
impl Tree {
    /// The tree of packages laid out on disk, each named and rooted at its
    /// directory: every import of every graph resolved from its file the way
    /// the module graph resolves it, and named by the package whose root
    /// holds the resolved file most closely. An import resolving into no
    /// package, or into its own, is left out.
    pub(crate) fn on_disk(packages: &[(&str, &std::path::Path, &PackageGraph)]) -> Self {
        let resolver = crate::ts::module_graph::DiskResolver::new();
        let mut tree = Self::default();
        for (name, root, graph) in packages {
            for import in &graph.boundary.imports {
                let Some(resolved) = resolver.resolve(&root.join(&import.file), &import.specifier)
                else {
                    continue;
                };
                let Some((target, root)) = packages
                    .iter()
                    .filter(|(_, root, _)| resolved.starts_with(root))
                    .max_by_key(|(_, root, _)| root.components().count())
                    .map(|(target, root, _)| (*target, *root))
                else {
                    continue;
                };
                let Ok(inner) = resolved.strip_prefix(root) else {
                    continue;
                };
                if target == *name {
                    continue;
                }
                let inner = inner
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                tree.links.insert(
                    (
                        (*name).to_string(),
                        import.file.clone(),
                        import.specifier.clone(),
                    ),
                    (target.to_string(), inner),
                );
            }
        }
        tree
    }
}

/// The answers of a stitched program. Nodes are named `<package>\0<text>`
/// and numbered by their position in `nodes`, which is sorted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Stitched {
    pub(crate) nodes: Vec<String>,
    pub(crate) sites: Vec<SiteEntry>,
    /// The imports the stitch did not link, with the reason.
    pub(crate) open_imports: Vec<OpenImport>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct OpenImport {
    pub(crate) package: String,
    pub(crate) file: String,
    pub(crate) specifier: String,
    pub(crate) reason: &'static str,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum StitchError {
    #[error("the solve of package `{package}` was cut short by its budget: {events:?}")]
    BudgetExceeded {
        package: String,
        events: Vec<String>,
    },
    #[error("two package graphs are named `{0}`")]
    DuplicatePackage(String),
}

/// Stitches `graphs`, each named by its package, along the links of `tree`.
/// The answer does not depend on the order of `graphs`.
pub(crate) fn stitch(
    graphs: &[(&str, &PackageGraph)],
    tree: &Tree,
) -> Result<Stitched, StitchError> {
    let mut graphs = graphs.to_vec();
    graphs.sort_by_key(|(name, _)| *name);
    if let Some(pair) = graphs.windows(2).find(|pair| pair[0].0 == pair[1].0) {
        return Err(StitchError::DuplicatePackage(pair[0].0.to_string()));
    }
    if let Some((name, graph)) = graphs
        .iter()
        .find(|(_, graph)| !graph.budget.events.is_empty())
    {
        return Err(StitchError::BudgetExceeded {
            package: (*name).to_string(),
            events: graph.budget.events.clone(),
        });
    }

    let texts = graphs
        .iter()
        .flat_map(|(name, graph)| graph.nodes.iter().map(move |text| node_text(name, text)))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let index = texts
        .iter()
        .enumerate()
        .map(|(at, text)| (text.as_str(), at as u64))
        .collect::<BTreeMap<_, _>>();
    // The global node of each graph's node numbers.
    let global = graphs
        .iter()
        .map(|(name, graph)| {
            graph
                .nodes
                .iter()
                .map(|text| SemanticNodeId(index[node_text(name, text).as_str()]))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let by_name = graphs
        .iter()
        .enumerate()
        .map(|(at, (name, _))| (*name, at))
        .collect::<BTreeMap<_, _>>();

    // The open copy of each linked import, and the place it is linked to;
    // `None` when the file it names holds no place for the value.
    let mut links = BTreeMap::new();
    let mut open_imports = Vec::new();
    for (at, (name, graph)) in graphs.iter().enumerate() {
        for import in &graph.boundary.imports {
            let node = |number: u32| global[at][number as usize];
            match link_source(&graphs, &by_name, &global, tree, name, import) {
                Ok(source) => {
                    links.insert((node(import.place), node(import.open)), source);
                }
                Err(reason) => open_imports.push(OpenImport {
                    package: (*name).to_string(),
                    file: import.file.clone(),
                    specifier: import.specifier.clone(),
                    reason,
                }),
            }
        }
    }
    let guard_arguments = graphs
        .iter()
        .map(|(_, graph)| graph.boundary.guard_arguments)
        .max()
        .unwrap_or(0);

    let mut entries = Vec::new();
    for (at, (name, graph)) in graphs.iter().enumerate() {
        for constraint in &graph.constraints {
            let mut kind = constraint.kind.clone();
            kind.remap_nodes(|node| global[at][ConstraintEntry::node(node) as usize]);
            if let ConstraintKind::CopyEdge { dst, src } = kind
                && let Some(Some(source)) = links.get(&(dst, src))
            {
                entries.push(ConstraintEntry {
                    kind: ConstraintKind::CopyEdge { dst, src: *source },
                    status: constraint.status,
                    precision: constraint.precision,
                    key: node_text(name, &format!("{}#link", constraint.key)),
                });
            }
            if constraint.key.starts_with(GUARD_KEY)
                && let ConstraintKind::CallConstraint {
                    receiver: Some(open),
                    arguments,
                    ..
                } = &mut kind
            {
                *arguments = vec![*open; usize::from(guard_arguments)];
            }
            entries.push(ConstraintEntry {
                kind,
                status: constraint.status,
                precision: constraint.precision,
                key: node_text(name, &constraint.key),
            });
        }
    }
    entries.sort();
    entries.dedup();

    let interner = StableKeyInterner::default();
    let key_of = |node: SemanticNodeId| interner.intern(texts[node.0 as usize].as_str());
    let constraints = entries
        .into_iter()
        .enumerate()
        .map(|(at, entry)| ConstraintFact {
            id: SemanticConstraintId(at as u64),
            kind: entry.kind,
            status: entry.status,
            precision: entry.precision,
            stable_key: interner.intern(entry.key),
        })
        .collect::<Vec<_>>();
    let mut inputs = TsPointsToInputs {
        invocations: invocations(&constraints),
        constraints,
        ..TsPointsToInputs::default()
    };
    for (at, (name, graph)) in graphs.iter().enumerate() {
        let node = |number: u32| global[at][number as usize];
        for function in &graph.functions {
            let function = node(function.node);
            inputs
                .callable_objects
                .insert(object_for_node(function), (function, key_of(function)));
        }
        for site in &graph.callsites {
            let callsite = node(site.node);
            inputs.callsites.push(TsPointsToCallsite {
                caller_node: node(site.caller),
                callsite_node: callsite,
                callsite_stable_key: key_of(callsite),
                constraint_stable_key: interner.intern(node_text(name, &site.constraint)),
                precision: site.precision,
            });
        }
        for signature in &graph.signatures {
            let place = |number: u32| SignaturePlace {
                var: var_for_node(node(number)),
                stable_key: key_of(node(number)),
            };
            inputs.signatures.insert(
                object_for_node(node(signature.function)),
                PointsToSignature {
                    params: signature
                        .params
                        .iter()
                        .map(|(position, number)| (*position, place(*number)))
                        .collect(),
                    rest: signature.rest.map(place),
                    this: signature.this.map(place),
                    ret: signature.ret.map(place),
                },
            );
        }
    }
    inputs.callsites.sort_by_key(|site| site.callsite_node);

    let result = solve_ts_points_to(&interner, &inputs, &SolverBudget::default());
    let number = |node: SemanticNodeId| u32::try_from(node.0).expect("node count fits u32");
    let sites = site_answers(&inputs, &result)
        .into_iter()
        .map(|answer| SiteEntry {
            callsite: number(answer.callsite),
            status: answer.status,
            claim: answer.claim,
            targets: answer
                .targets
                .iter()
                .map(|target| number(*target))
                .collect(),
        })
        .collect();
    open_imports.sort();
    Ok(Stitched {
        nodes: texts,
        sites,
        open_imports,
    })
}

fn node_text(package: &str, text: &str) -> String {
    format!("{package}\0{text}")
}

/// The global place an import is linked to: `Ok(None)` when the file it
/// names holds no place for the value, so the import receives nothing, and
/// the reason when it cannot be linked.
fn link_source(
    graphs: &[(&str, &PackageGraph)],
    by_name: &BTreeMap<&str, usize>,
    global: &[Vec<SemanticNodeId>],
    tree: &Tree,
    package: &str,
    import: &ImportEntry,
) -> Result<Option<SemanticNodeId>, &'static str> {
    let key = (
        package.to_string(),
        import.file.clone(),
        import.specifier.clone(),
    );
    let Some((target, file)) = tree.links.get(&key) else {
        return Err("unlinked_import");
    };
    let Some(&at) = by_name.get(target.as_str()) else {
        return Err("missing_package_graph");
    };
    let modules = &graphs[at].1.boundary.modules;
    let Ok(found) = modules.binary_search_by(|entry| entry.file.as_str().cmp(file)) else {
        return Ok(None);
    };
    let place = match import.value {
        ModuleValue::Module => modules[found].module,
        ModuleValue::Namespace => modules[found].namespace,
    };
    Ok(place.map(|number| global[at][number as usize]))
}

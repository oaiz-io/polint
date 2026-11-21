//! Stitches the package graphs of a program into the answers of the whole
//! program.
//!
//! The steps:
//!
//! 1. Refuse a graph whose solve its budget cut short: it is not a fixed point.
//! 2. Name every node by its package and its text, so the graphs share no node.
//!    A package is one install: two versions of one package in a tree are two
//!    packages with two names.
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
use crate::analysis_api::ImportSyntax;
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

/// Where the imports of a program lead: for each package, importing file,
/// specifier and syntax, the files of other packages the import may name.
#[derive(Debug, Clone, Default)]
pub(crate) struct Tree {
    pub(crate) links: BTreeMap<(String, String, String, ImportSyntax), Link>,
}

/// The files of other packages one import may name, each as its package and
/// the path inside it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Link {
    pub(crate) files: BTreeSet<(String, String)>,
    /// Whether the ways of loading the import name different files, so no
    /// one file is its target.
    pub(crate) alternatives: bool,
}

#[cfg(feature = "lang-typescript")]
impl Tree {
    /// The tree of packages laid out on disk, each named and rooted at its
    /// install directory: every import of every graph resolved from its file
    /// the way the module graph resolves it, and each file it may name
    /// named by the package whose root holds the file most closely. A file
    /// in no package, or in the importing one, is left out, and so is an
    /// import that names one file and has none left.
    pub(crate) fn on_disk(packages: &[(&str, &std::path::Path, &PackageGraph)]) -> Self {
        use crate::ts::module_graph::DiskTarget;
        let resolver = crate::ts::module_graph::DiskResolver::new();
        let mut tree = Self::default();
        for (name, root, graph) in packages {
            for import in &graph.boundary.imports {
                let (files, alternatives) = match resolver.resolve(
                    &root.join(&import.file),
                    &import.specifier,
                    import.syntax,
                ) {
                    Some(DiskTarget::File(file)) => (vec![file], false),
                    Some(DiskTarget::Alternatives(files)) => (files, true),
                    None => continue,
                };
                let files = files
                    .iter()
                    .filter_map(|resolved| {
                        let (target, root) = packages
                            .iter()
                            .filter(|(_, root, _)| resolved.starts_with(root))
                            .max_by_key(|(_, root, _)| root.components().count())
                            .map(|(target, root, _)| (*target, *root))?;
                        let inner = resolved.strip_prefix(root).ok()?;
                        (target != *name).then(|| {
                            let inner = inner
                                .components()
                                .map(|part| part.as_os_str().to_string_lossy())
                                .collect::<Vec<_>>()
                                .join("/");
                            (target.to_string(), inner)
                        })
                    })
                    .collect::<BTreeSet<_>>();
                if files.is_empty() && !alternatives {
                    continue;
                }
                tree.links.insert(
                    (
                        (*name).to_string(),
                        import.file.clone(),
                        import.specifier.clone(),
                        import.syntax,
                    ),
                    Link {
                        files,
                        alternatives,
                    },
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

    // The open copy of each linked import, and the places it is linked to.
    let mut links = BTreeMap::<_, Vec<SemanticNodeId>>::new();
    let mut open_imports = Vec::new();
    for (at, (name, graph)) in graphs.iter().enumerate() {
        for import in &graph.boundary.imports {
            let node = |number: u32| global[at][number as usize];
            let open = |reason| OpenImport {
                package: (*name).to_string(),
                file: import.file.clone(),
                specifier: import.specifier.clone(),
                reason,
            };
            let key = (
                (*name).to_string(),
                import.file.clone(),
                import.specifier.clone(),
                import.syntax,
            );
            let Some(link) = tree.links.get(&key) else {
                open_imports.push(open("unlinked_import"));
                continue;
            };
            if link.alternatives {
                open_imports.push(open("no_single_target"));
            }
            let sources = links
                .entry((node(import.place), node(import.open)))
                .or_default();
            for (target, file) in &link.files {
                match link_source(&graphs, &by_name, &global, target, file, import.value) {
                    Ok(source) => sources.extend(source),
                    Err(reason) => open_imports.push(open(reason)),
                }
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
            if let ConstraintKind::CopyEdge { dst, src } = kind {
                for (link, &source) in links.get(&(dst, src)).into_iter().flatten().enumerate() {
                    entries.push(ConstraintEntry {
                        kind: ConstraintKind::CopyEdge { dst, src: source },
                        status: constraint.status,
                        precision: constraint.precision,
                        key: node_text(name, &format!("{}#link#{link}", constraint.key)),
                    });
                }
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
    open_imports.dedup();
    Ok(Stitched {
        nodes: texts,
        sites,
        open_imports,
    })
}

fn node_text(package: &str, text: &str) -> String {
    format!("{package}\0{text}")
}

/// The global place an import of `value` is linked to in `file` of the
/// package `target`: `Ok(None)` when the file holds no place for the value,
/// so the import receives nothing from it, and the reason when it cannot be
/// linked.
fn link_source(
    graphs: &[(&str, &PackageGraph)],
    by_name: &BTreeMap<&str, usize>,
    global: &[Vec<SemanticNodeId>],
    target: &str,
    file: &str,
    value: ModuleValue,
) -> Result<Option<SemanticNodeId>, &'static str> {
    let Some(&at) = by_name.get(target) else {
        return Err("missing_package_graph");
    };
    let modules = &graphs[at].1.boundary.modules;
    let Ok(found) = modules.binary_search_by(|entry| entry.file.as_str().cmp(file)) else {
        return Ok(None);
    };
    let place = match value {
        ModuleValue::Module => modules[found].module,
        ModuleValue::Namespace => modules[found].namespace,
    };
    Ok(place.map(|number| global[at][number as usize]))
}

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::facts::{
    EvidenceBundleFact, EvidenceEdgeFact, EvidenceEdgeKind, EvidenceNodeFact, EvidenceNodeKind,
    EvidenceOmittedRegionFact, EvidencePathFact, EvidenceProvenance, EvidenceQueryMode,
    EvidenceReplayKeyFact, EvidenceSliceFact, EvidenceStatus, EvidenceUnknownFact,
};
use crate::analysis_neutral::error::AnalysisError;
use crate::analysis_neutral::ids::{
    EvidenceBundleId, EvidenceEdgeId, EvidenceNodeId, EvidenceOmittedRegionId, EvidencePathId,
    EvidenceSliceId,
};
use crate::internal_core::{StableKeyId, StableKeyInterner};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EvidenceOutput {
    pub nodes: Vec<EvidenceNodeFact>,
    pub edges: Vec<EvidenceEdgeFact>,
    pub bundles: Vec<EvidenceBundleFact>,
    pub paths: Vec<EvidencePathFact>,
    pub slices: Vec<EvidenceSliceFact>,
    pub unknowns: Vec<EvidenceUnknownFact>,
    pub omitted_regions: Vec<EvidenceOmittedRegionFact>,
    pub replay_keys: Vec<EvidenceReplayKeyFact>,
}

impl EvidenceOutput {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn normalized(mut self, interner: &StableKeyInterner) -> Self {
        self.nodes = normalize_nodes(self.nodes, interner);
        self.omitted_regions = normalize_omitted_regions(self.omitted_regions, interner);
        self.bundles = normalize_bundles(self.bundles, interner);
        self.paths = self
            .paths
            .into_iter()
            .map(EvidencePathFact::normalized)
            .collect();
        self.slices = self
            .slices
            .into_iter()
            .map(EvidenceSliceFact::normalized)
            .collect();
        self.unknowns = self
            .unknowns
            .into_iter()
            .map(EvidenceUnknownFact::normalized)
            .collect();
        self.replay_keys = self
            .replay_keys
            .into_iter()
            .map(EvidenceReplayKeyFact::normalized)
            .collect();

        let node_remap = self
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.id, EvidenceNodeId(index as u64)))
            .collect::<BTreeMap<_, _>>();
        let omitted_remap = self
            .omitted_regions
            .iter()
            .enumerate()
            .map(|(index, omitted)| (omitted.id, EvidenceOmittedRegionId(index as u64)))
            .collect::<BTreeMap<_, _>>();
        let bundle_remap = self
            .bundles
            .iter()
            .enumerate()
            .map(|(index, bundle)| (bundle.id, EvidenceBundleId(index as u64)))
            .collect::<BTreeMap<_, _>>();

        for (index, node) in self.nodes.iter_mut().enumerate() {
            node.id = EvidenceNodeId(index as u64);
        }
        for (index, omitted) in self.omitted_regions.iter_mut().enumerate() {
            omitted.id = EvidenceOmittedRegionId(index as u64);
            remap_option(&mut omitted.bundle, &bundle_remap);
        }
        for (index, bundle) in self.bundles.iter_mut().enumerate() {
            bundle.id = EvidenceBundleId(index as u64);
            remap_option(&mut bundle.entry_node, &node_remap);
        }

        self.edges = self
            .edges
            .into_iter()
            .map(EvidenceEdgeFact::normalized)
            .map(|mut edge| {
                remap_required(&mut edge.from, &node_remap);
                remap_required(&mut edge.to, &node_remap);
                edge
            })
            .collect();
        self.edges.sort_by(|left, right| {
            interner
                .compare_canonical(left.stable_key, right.stable_key)
                .then_with(|| left.from.cmp(&right.from))
                .then_with(|| left.to.cmp(&right.to))
                .then_with(|| left.kind.cmp(&right.kind))
                .then_with(|| left.query_mode.cmp(&right.query_mode))
                .then_with(|| left.id.cmp(&right.id))
        });
        let edge_remap = self
            .edges
            .iter()
            .enumerate()
            .map(|(index, edge)| (edge.id, EvidenceEdgeId(index as u64)))
            .collect::<BTreeMap<_, _>>();
        for (index, edge) in self.edges.iter_mut().enumerate() {
            edge.id = EvidenceEdgeId(index as u64);
        }

        self.paths = self
            .paths
            .into_iter()
            .map(|mut path| {
                remap_option(&mut path.bundle, &bundle_remap);
                remap_vec(&mut path.nodes, &node_remap);
                remap_vec(&mut path.edges, &edge_remap);
                remap_vec(&mut path.omitted_regions, &omitted_remap);
                path
            })
            .collect();
        self.paths.sort_by(|left, right| {
            interner
                .compare_canonical(left.stable_key, right.stable_key)
                .then_with(|| left.rank.cmp(&right.rank))
                .then_with(|| left.id.cmp(&right.id))
        });
        let path_remap = self
            .paths
            .iter()
            .enumerate()
            .map(|(index, path)| (path.id, EvidencePathId(index as u64)))
            .collect::<BTreeMap<_, _>>();
        for (index, path) in self.paths.iter_mut().enumerate() {
            path.id = EvidencePathId(index as u64);
        }

        self.slices = self
            .slices
            .into_iter()
            .map(|mut slice| {
                remap_option(&mut slice.bundle, &bundle_remap);
                remap_vec(&mut slice.root_nodes, &node_remap);
                remap_vec(&mut slice.nodes, &node_remap);
                remap_vec(&mut slice.edges, &edge_remap);
                remap_vec(&mut slice.omitted_regions, &omitted_remap);
                slice
            })
            .collect();
        self.slices.sort_by(|left, right| {
            interner
                .compare_canonical(left.stable_key, right.stable_key)
                .then_with(|| left.query_mode.cmp(&right.query_mode))
                .then_with(|| left.id.cmp(&right.id))
        });
        let slice_remap = self
            .slices
            .iter()
            .enumerate()
            .map(|(index, slice)| (slice.id, EvidenceSliceId(index as u64)))
            .collect::<BTreeMap<_, _>>();
        for (index, slice) in self.slices.iter_mut().enumerate() {
            slice.id = EvidenceSliceId(index as u64);
        }

        for bundle in &mut self.bundles {
            remap_vec(&mut bundle.selected_paths, &path_remap);
            remap_vec(&mut bundle.selected_slices, &slice_remap);
        }
        for omitted in &mut self.omitted_regions {
            remap_option(&mut omitted.path, &path_remap);
            remap_option(&mut omitted.slice, &slice_remap);
        }
        for unknown in &mut self.unknowns {
            remap_option(&mut unknown.bundle, &bundle_remap);
            remap_option(&mut unknown.path, &path_remap);
            remap_option(&mut unknown.slice, &slice_remap);
            remap_option(&mut unknown.edge, &edge_remap);
        }
        for replay in &mut self.replay_keys {
            remap_required(&mut replay.bundle, &bundle_remap);
        }

        self.unknowns.sort_by(|left, right| {
            interner
                .compare_canonical(left.stable_key, right.stable_key)
                .then_with(|| left.reason.cmp(&right.reason))
        });
        self.replay_keys.sort_by(|left, right| {
            interner
                .compare_canonical(left.stable_key, right.stable_key)
                .then_with(|| left.query_mode.cmp(&right.query_mode))
        });

        self
    }
}

#[derive(Debug, Clone, Default)]
pub struct EvidenceStore {
    output: EvidenceOutput,
    interner: StableKeyInterner,
    by_node_kind: BTreeMap<EvidenceNodeKind, Vec<usize>>,
    by_edge_kind: BTreeMap<EvidenceEdgeKind, Vec<usize>>,
    by_bundle: BTreeMap<EvidenceBundleId, Vec<usize>>,
    by_diagnostic_stable_key: BTreeMap<StableKeyId, Vec<usize>>,
    by_source_fact_stable_key: BTreeMap<Arc<str>, Vec<usize>>,
    by_query_mode: BTreeMap<EvidenceQueryMode, Vec<usize>>,
    by_status: BTreeMap<EvidenceStatus, Vec<usize>>,
    by_provenance: BTreeMap<EvidenceProvenance, Vec<usize>>,
    by_replay_key: BTreeMap<String, Vec<usize>>,
    outgoing: BTreeMap<EvidenceNodeId, Vec<usize>>,
    incoming: BTreeMap<EvidenceNodeId, Vec<usize>>,
}

impl EvidenceStore {
    pub fn from_output(
        output: EvidenceOutput,
        interner: &StableKeyInterner,
    ) -> Result<Self, AnalysisError> {
        validate_references(&output, interner)?;
        let output = output.normalized(interner);
        validate_references(&output, interner)?;
        reject_duplicates(
            interner,
            output.nodes.iter().map(|node| node.stable_key),
            "evidence node stable key",
        )?;
        reject_duplicates(
            interner,
            output.edges.iter().map(|edge| edge.stable_key),
            "evidence edge stable key",
        )?;
        reject_duplicates(
            interner,
            output.bundles.iter().map(|bundle| bundle.stable_key),
            "evidence bundle stable key",
        )?;
        reject_duplicates(
            interner,
            output.paths.iter().map(|path| path.stable_key),
            "evidence path stable key",
        )?;
        reject_duplicates(
            interner,
            output.slices.iter().map(|slice| slice.stable_key),
            "evidence slice stable key",
        )?;
        reject_duplicates(
            interner,
            output
                .omitted_regions
                .iter()
                .map(|omitted| omitted.stable_key),
            "evidence omitted-region stable key",
        )?;
        reject_duplicates(
            interner,
            output.unknowns.iter().map(|unknown| unknown.stable_key),
            "evidence unknown stable key",
        )?;
        reject_duplicates(
            interner,
            output.replay_keys.iter().map(|replay| replay.stable_key),
            "evidence replay key stable key",
        )?;

        let mut store = Self {
            output,
            interner: interner.clone(),
            ..Self::default()
        };
        for (index, node) in store.output.nodes.iter().enumerate() {
            store.by_node_kind.entry(node.kind).or_default().push(index);
            store.by_status.entry(node.status).or_default().push(index);
            store
                .by_provenance
                .entry(node.provenance)
                .or_default()
                .push(index);
            for source_key in &node.source_fact_stable_keys {
                store
                    .by_source_fact_stable_key
                    .entry(source_key.clone())
                    .or_default()
                    .push(index);
            }
        }
        for (index, edge) in store.output.edges.iter().enumerate() {
            store.by_edge_kind.entry(edge.kind).or_default().push(index);
            store
                .by_query_mode
                .entry(edge.query_mode)
                .or_default()
                .push(index);
            store.by_status.entry(edge.status).or_default().push(index);
            store
                .by_provenance
                .entry(edge.provenance)
                .or_default()
                .push(index);
            for source_key in &edge.source_fact_stable_keys {
                store
                    .by_source_fact_stable_key
                    .entry(source_key.clone())
                    .or_default()
                    .push(index);
            }
            store.outgoing.entry(edge.from).or_default().push(index);
            store.incoming.entry(edge.to).or_default().push(index);
        }
        for (index, bundle) in store.output.bundles.iter().enumerate() {
            store
                .by_diagnostic_stable_key
                .entry(bundle.diagnostic_stable_key)
                .or_default()
                .push(index);
            store
                .by_query_mode
                .entry(bundle.query_mode)
                .or_default()
                .push(index);
            if let Some(replay_key) = &bundle.replay_key {
                store
                    .by_replay_key
                    .entry(replay_key.clone())
                    .or_default()
                    .push(index);
            }
        }
        for (index, path) in store.output.paths.iter().enumerate() {
            if let Some(bundle) = path.bundle {
                store.by_bundle.entry(bundle).or_default().push(index);
            }
        }
        Ok(store)
    }

    pub fn nodes(&self) -> &[EvidenceNodeFact] {
        &self.output.nodes
    }

    pub fn edges(&self) -> &[EvidenceEdgeFact] {
        &self.output.edges
    }

    pub fn bundles(&self) -> &[EvidenceBundleFact] {
        &self.output.bundles
    }

    pub fn paths(&self) -> &[EvidencePathFact] {
        &self.output.paths
    }

    pub fn slices(&self) -> &[EvidenceSliceFact] {
        &self.output.slices
    }

    pub fn unknowns(&self) -> &[EvidenceUnknownFact] {
        &self.output.unknowns
    }

    pub fn omitted_regions(&self) -> &[EvidenceOmittedRegionFact] {
        &self.output.omitted_regions
    }

    pub fn replay_keys(&self) -> &[EvidenceReplayKeyFact] {
        &self.output.replay_keys
    }

    pub fn resolve_stable_key(&self, key: StableKeyId) -> std::sync::Arc<str> {
        self.interner.resolve(key)
    }

    pub fn node(&self, node: EvidenceNodeId) -> Option<&EvidenceNodeFact> {
        self.output.nodes.get(node.0 as usize)
    }

    pub fn edge(&self, edge: EvidenceEdgeId) -> Option<&EvidenceEdgeFact> {
        self.output.edges.get(edge.0 as usize)
    }

    pub fn incoming(&self, node: EvidenceNodeId) -> Vec<&EvidenceEdgeFact> {
        self.edge_refs(self.incoming.get(&node))
    }

    pub fn outgoing(&self, node: EvidenceNodeId) -> Vec<&EvidenceEdgeFact> {
        self.edge_refs(self.outgoing.get(&node))
    }

    pub fn by_edge_kind(&self, kind: EvidenceEdgeKind) -> Vec<&EvidenceEdgeFact> {
        self.edge_refs(self.by_edge_kind.get(&kind))
    }

    fn edge_refs(&self, indexes: Option<&Vec<usize>>) -> Vec<&EvidenceEdgeFact> {
        indexes.map_or_else(Vec::new, |indexes| {
            indexes
                .iter()
                .map(|index| &self.output.edges[*index])
                .collect()
        })
    }
}

fn normalize_nodes(
    mut nodes: Vec<EvidenceNodeFact>,
    interner: &StableKeyInterner,
) -> Vec<EvidenceNodeFact> {
    nodes = nodes
        .into_iter()
        .map(EvidenceNodeFact::normalized)
        .collect();
    nodes.sort_by(|left, right| {
        interner
            .compare_canonical(left.stable_key, right.stable_key)
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.id.cmp(&right.id))
    });
    nodes
}

fn normalize_omitted_regions(
    mut omitted_regions: Vec<EvidenceOmittedRegionFact>,
    interner: &StableKeyInterner,
) -> Vec<EvidenceOmittedRegionFact> {
    omitted_regions.sort_by(|left, right| {
        interner
            .compare_canonical(left.stable_key, right.stable_key)
            .then_with(|| left.reason.cmp(&right.reason))
            .then_with(|| left.id.cmp(&right.id))
    });
    omitted_regions
}

fn normalize_bundles(
    bundles: Vec<EvidenceBundleFact>,
    interner: &StableKeyInterner,
) -> Vec<EvidenceBundleFact> {
    let mut bundles = bundles
        .into_iter()
        .map(EvidenceBundleFact::normalized)
        .collect::<Vec<_>>();
    bundles.sort_by(|left, right| {
        interner
            .compare_canonical(left.stable_key, right.stable_key)
            .then_with(|| left.query_mode.cmp(&right.query_mode))
            .then_with(|| left.id.cmp(&right.id))
    });
    bundles
}

fn validate_references(
    output: &EvidenceOutput,
    interner: &StableKeyInterner,
) -> Result<(), AnalysisError> {
    reject_duplicate_ids(output.nodes.iter().map(|node| node.id), "evidence node id")?;
    reject_duplicate_ids(output.edges.iter().map(|edge| edge.id), "evidence edge id")?;
    reject_duplicate_ids(
        output.bundles.iter().map(|bundle| bundle.id),
        "evidence bundle id",
    )?;
    reject_duplicate_ids(output.paths.iter().map(|path| path.id), "evidence path id")?;
    reject_duplicate_ids(
        output.slices.iter().map(|slice| slice.id),
        "evidence slice id",
    )?;
    reject_duplicate_ids(
        output.omitted_regions.iter().map(|omitted| omitted.id),
        "evidence omitted-region id",
    )?;

    let nodes = output
        .nodes
        .iter()
        .map(|node| node.id)
        .collect::<BTreeSet<_>>();
    let edges = output
        .edges
        .iter()
        .map(|edge| edge.id)
        .collect::<BTreeSet<_>>();
    let bundles = output
        .bundles
        .iter()
        .map(|bundle| bundle.id)
        .collect::<BTreeSet<_>>();
    let paths = output
        .paths
        .iter()
        .map(|path| path.id)
        .collect::<BTreeSet<_>>();
    let slices = output
        .slices
        .iter()
        .map(|slice| slice.id)
        .collect::<BTreeSet<_>>();
    let omitted = output
        .omitted_regions
        .iter()
        .map(|region| region.id)
        .collect::<BTreeSet<_>>();

    for edge in &output.edges {
        if !nodes.contains(&edge.from) || !nodes.contains(&edge.to) {
            return invalid(format!(
                "evidence edge `{}` references missing endpoint",
                interner.resolve(edge.stable_key)
            ));
        }
    }
    for bundle in &output.bundles {
        if bundle.entry_node.is_some_and(|node| !nodes.contains(&node)) {
            return invalid(format!(
                "evidence bundle `{}` references missing entry node",
                interner.resolve(bundle.stable_key)
            ));
        }
        require_all(
            &bundle.selected_paths,
            &paths,
            "path",
            interner.resolve(bundle.stable_key).as_ref(),
            "bundle",
        )?;
        require_all(
            &bundle.selected_slices,
            &slices,
            "slice",
            interner.resolve(bundle.stable_key).as_ref(),
            "bundle",
        )?;
    }
    for path in &output.paths {
        require_option(
            path.bundle,
            &bundles,
            "bundle",
            interner.resolve(path.stable_key).as_ref(),
            "path",
        )?;
        require_all(
            &path.nodes,
            &nodes,
            "node",
            interner.resolve(path.stable_key).as_ref(),
            "path",
        )?;
        require_all(
            &path.edges,
            &edges,
            "edge",
            interner.resolve(path.stable_key).as_ref(),
            "path",
        )?;
        require_all(
            &path.omitted_regions,
            &omitted,
            "omitted region",
            interner.resolve(path.stable_key).as_ref(),
            "path",
        )?;
    }
    for slice in &output.slices {
        require_option(
            slice.bundle,
            &bundles,
            "bundle",
            interner.resolve(slice.stable_key).as_ref(),
            "slice",
        )?;
        require_all(
            &slice.root_nodes,
            &nodes,
            "root node",
            interner.resolve(slice.stable_key).as_ref(),
            "slice",
        )?;
        require_all(
            &slice.nodes,
            &nodes,
            "node",
            interner.resolve(slice.stable_key).as_ref(),
            "slice",
        )?;
        require_all(
            &slice.edges,
            &edges,
            "edge",
            interner.resolve(slice.stable_key).as_ref(),
            "slice",
        )?;
        require_all(
            &slice.omitted_regions,
            &omitted,
            "omitted region",
            interner.resolve(slice.stable_key).as_ref(),
            "slice",
        )?;
    }
    for unknown in &output.unknowns {
        require_option(
            unknown.bundle,
            &bundles,
            "bundle",
            interner.resolve(unknown.stable_key).as_ref(),
            "unknown",
        )?;
        require_option(
            unknown.path,
            &paths,
            "path",
            interner.resolve(unknown.stable_key).as_ref(),
            "unknown",
        )?;
        require_option(
            unknown.slice,
            &slices,
            "slice",
            interner.resolve(unknown.stable_key).as_ref(),
            "unknown",
        )?;
        require_option(
            unknown.edge,
            &edges,
            "edge",
            interner.resolve(unknown.stable_key).as_ref(),
            "unknown",
        )?;
    }
    for region in &output.omitted_regions {
        require_option(
            region.bundle,
            &bundles,
            "bundle",
            interner.resolve(region.stable_key).as_ref(),
            "omitted",
        )?;
        require_option(
            region.path,
            &paths,
            "path",
            interner.resolve(region.stable_key).as_ref(),
            "omitted",
        )?;
        require_option(
            region.slice,
            &slices,
            "slice",
            interner.resolve(region.stable_key).as_ref(),
            "omitted",
        )?;
    }
    for replay in &output.replay_keys {
        require_one(
            replay.bundle,
            &bundles,
            "bundle",
            interner.resolve(replay.stable_key).as_ref(),
            "replay",
        )?;
    }

    Ok(())
}

fn require_all<T>(
    values: &[T],
    allowed: &BTreeSet<T>,
    label: &'static str,
    owner_key: &str,
    owner_kind: &'static str,
) -> Result<(), AnalysisError>
where
    T: Copy + Ord,
{
    for value in values {
        require_one(*value, allowed, label, owner_key, owner_kind)?;
    }
    Ok(())
}

fn require_option<T>(
    value: Option<T>,
    allowed: &BTreeSet<T>,
    label: &'static str,
    owner_key: &str,
    owner_kind: &'static str,
) -> Result<(), AnalysisError>
where
    T: Copy + Ord,
{
    if let Some(value) = value {
        require_one(value, allowed, label, owner_key, owner_kind)?;
    }
    Ok(())
}

fn require_one<T>(
    value: T,
    allowed: &BTreeSet<T>,
    label: &'static str,
    owner_key: &str,
    owner_kind: &'static str,
) -> Result<(), AnalysisError>
where
    T: Ord,
{
    if allowed.contains(&value) {
        Ok(())
    } else {
        invalid(format!(
            "evidence {owner_kind} `{owner_key}` references missing {label}"
        ))
    }
}

fn reject_duplicate_ids<T>(
    ids: impl Iterator<Item = T>,
    label: &'static str,
) -> Result<(), AnalysisError>
where
    T: Copy + Ord + std::fmt::Debug,
{
    let mut seen = BTreeSet::new();
    for id in ids {
        if !seen.insert(id) {
            return invalid(format!("duplicate {label} `{id:?}`"));
        }
    }
    Ok(())
}

fn reject_duplicates(
    interner: &StableKeyInterner,
    keys: impl Iterator<Item = StableKeyId>,
    label: &'static str,
) -> Result<(), AnalysisError> {
    let mut seen = BTreeSet::new();
    for key in keys {
        if !seen.insert(key) {
            return invalid(format!("duplicate {label} `{}`", interner.resolve(key)));
        }
    }
    Ok(())
}

fn invalid<T>(reason: String) -> Result<T, AnalysisError> {
    Err(AnalysisError::InvalidFact {
        provider: "polint.evidence",
        reason,
    })
}

fn remap_option<T>(value: &mut Option<T>, remap: &BTreeMap<T, T>)
where
    T: Copy + Ord,
{
    if let Some(remapped) = value.and_then(|id| remap.get(&id).copied()) {
        *value = Some(remapped);
    }
}

fn remap_required<T>(value: &mut T, remap: &BTreeMap<T, T>)
where
    T: Copy + Ord,
{
    if let Some(remapped) = remap.get(value).copied() {
        *value = remapped;
    }
}

fn remap_vec<T>(values: &mut Vec<T>, remap: &BTreeMap<T, T>)
where
    T: Copy + Ord,
{
    for value in values {
        remap_required(value, remap);
    }
}

pub fn next_evidence_node_id(nodes: &[EvidenceNodeFact]) -> EvidenceNodeId {
    EvidenceNodeId(nodes.len() as u64)
}

pub fn next_evidence_edge_id(edges: &[EvidenceEdgeFact]) -> EvidenceEdgeId {
    EvidenceEdgeId(edges.len() as u64)
}

pub fn next_evidence_bundle_id(bundles: &[EvidenceBundleFact]) -> EvidenceBundleId {
    EvidenceBundleId(bundles.len() as u64)
}

pub fn next_evidence_path_id(paths: &[EvidencePathFact]) -> EvidencePathId {
    EvidencePathId(paths.len() as u64)
}

pub fn next_evidence_slice_id(slices: &[EvidenceSliceFact]) -> EvidenceSliceId {
    EvidenceSliceId(slices.len() as u64)
}

pub fn next_evidence_omitted_region_id(
    omitted_regions: &[EvidenceOmittedRegionFact],
) -> EvidenceOmittedRegionId {
    EvidenceOmittedRegionId(omitted_regions.len() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::evidence::facts::{
        EvidenceConfidence, EvidenceExpansion, EvidencePrecision, EvidenceRankScore,
        EvidenceUnknownReason, EvidenceValidation,
    };
    use crate::internal_core::Language;

    #[test]
    fn output_sorts_and_reassigns_ids_with_edge_endpoint_remap() {
        let interner = crate::internal_core::test_stable_key_interner();
        let output = EvidenceOutput {
            nodes: vec![node(9, "node:z"), node(3, "node:a")],
            edges: vec![edge(7, 9, 3, "edge:z_to_a")],
            bundles: Vec::new(),
            paths: Vec::new(),
            slices: Vec::new(),
            unknowns: Vec::new(),
            omitted_regions: Vec::new(),
            replay_keys: Vec::new(),
        }
        .normalized(&interner);

        assert_eq!(
            interner.resolve(output.nodes[0].stable_key).as_ref(),
            "node:a"
        );
        assert_eq!(output.nodes[0].id, EvidenceNodeId(0));
        assert_eq!(output.edges[0].from, EvidenceNodeId(1));
        assert_eq!(output.edges[0].to, EvidenceNodeId(0));
        assert_eq!(output.edges[0].id, EvidenceEdgeId(0));
    }

    #[test]
    fn store_rejects_dangling_edge_endpoints() {
        let result = EvidenceStore::from_output(
            EvidenceOutput {
                nodes: vec![node(0, "node:a")],
                edges: vec![edge(0, 0, 4, "edge:dangling")],
                bundles: Vec::new(),
                paths: Vec::new(),
                slices: Vec::new(),
                unknowns: Vec::new(),
                omitted_regions: Vec::new(),
                replay_keys: Vec::new(),
            },
            &crate::internal_core::test_stable_key_interner(),
        );

        assert!(result.is_err());
    }

    #[test]
    fn store_rejects_duplicate_node_stable_keys() {
        let result = EvidenceStore::from_output(
            EvidenceOutput {
                nodes: vec![node(0, "node:a"), node(1, "node:a")],
                edges: Vec::new(),
                bundles: Vec::new(),
                paths: Vec::new(),
                slices: Vec::new(),
                unknowns: Vec::new(),
                omitted_regions: Vec::new(),
                replay_keys: Vec::new(),
            },
            &crate::internal_core::test_stable_key_interner(),
        );

        assert!(result.is_err());
    }

    #[test]
    fn store_rejects_duplicate_unknown_stable_keys() {
        let result = EvidenceStore::from_output(
            EvidenceOutput {
                nodes: Vec::new(),
                edges: Vec::new(),
                bundles: Vec::new(),
                paths: Vec::new(),
                slices: Vec::new(),
                unknowns: vec![
                    EvidenceUnknownFact {
                        bundle: None,
                        path: None,
                        slice: None,
                        edge: None,
                        reason: EvidenceUnknownReason::OpaqueSummary,
                        message: "first".to_string(),
                        source_fact_stable_keys: Vec::new(),
                        stable_key: crate::internal_core::stable_key_for_test("unknown:dup"),
                    },
                    EvidenceUnknownFact {
                        bundle: None,
                        path: None,
                        slice: None,
                        edge: None,
                        reason: EvidenceUnknownReason::BudgetExceeded,
                        message: "second".to_string(),
                        source_fact_stable_keys: Vec::new(),
                        stable_key: crate::internal_core::stable_key_for_test("unknown:dup"),
                    },
                ],
                omitted_regions: Vec::new(),
                replay_keys: Vec::new(),
            },
            &crate::internal_core::test_stable_key_interner(),
        );

        assert!(result.is_err());
    }

    #[test]
    fn store_rejects_missing_bundle_path_slice_and_omitted_references() {
        let mut output = EvidenceOutput {
            nodes: vec![node(0, "node:a")],
            edges: Vec::new(),
            bundles: vec![EvidenceBundleFact {
                id: EvidenceBundleId(0),
                diagnostic_stable_key: crate::internal_core::stable_key_for_test("diagnostic:a"),
                query_mode: EvidenceQueryMode::ThinBackward,
                status: EvidenceStatus::Present,
                precision: EvidencePrecision::Syntax,
                provenance: EvidenceProvenance::Native,
                validation: EvidenceValidation::Native,
                confidence: EvidenceConfidence::High,
                entry_node: Some(EvidenceNodeId(0)),
                selected_paths: vec![EvidencePathId(9)],
                selected_slices: vec![EvidenceSliceId(9)],
                replay_key: None,
                stable_key: crate::internal_core::stable_key_for_test("bundle:a"),
            }],
            paths: vec![EvidencePathFact {
                id: EvidencePathId(0),
                bundle: Some(EvidenceBundleId(9)),
                query_mode: EvidenceQueryMode::Path,
                nodes: vec![EvidenceNodeId(0)],
                edges: Vec::new(),
                rank: 0,
                score: EvidenceRankScore::default(),
                status: EvidenceStatus::Present,
                hidden_node_count: 0,
                omitted_regions: vec![EvidenceOmittedRegionId(9)],
                stable_key: crate::internal_core::stable_key_for_test("path:a"),
            }],
            slices: Vec::new(),
            unknowns: Vec::new(),
            omitted_regions: Vec::new(),
            replay_keys: Vec::new(),
        };

        assert!(
            EvidenceStore::from_output(
                output.clone(),
                &crate::internal_core::test_stable_key_interner()
            )
            .is_err()
        );

        output.bundles[0].selected_paths.clear();
        output.bundles[0].selected_slices.clear();
        assert!(
            EvidenceStore::from_output(output, &crate::internal_core::test_stable_key_interner())
                .is_err()
        );
    }

    fn node(id: u64, stable_key: &str) -> EvidenceNodeFact {
        EvidenceNodeFact {
            id: EvidenceNodeId(id),
            kind: EvidenceNodeKind::Operation,
            language: Language::Go,
            file: None,
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
            stable_key: crate::internal_core::stable_key_for_test(stable_key),
        }
    }

    fn edge(id: u64, from: u64, to: u64, stable_key: &str) -> EvidenceEdgeFact {
        EvidenceEdgeFact {
            id: EvidenceEdgeId(id),
            from: EvidenceNodeId(from),
            to: EvidenceNodeId(to),
            kind: EvidenceEdgeKind::DataValue,
            query_mode: EvidenceQueryMode::ThinBackward,
            status: EvidenceStatus::Present,
            precision: EvidencePrecision::Syntax,
            provenance: EvidenceProvenance::Native,
            validation: EvidenceValidation::Native,
            confidence: EvidenceConfidence::High,
            call_site: None,
            summary_stable_key: None,
            expansion: EvidenceExpansion::None,
            compact_label: None,
            source_fact_stable_keys: Vec::new(),
            stable_key: crate::internal_core::stable_key_for_test(stable_key),
        }
    }
}

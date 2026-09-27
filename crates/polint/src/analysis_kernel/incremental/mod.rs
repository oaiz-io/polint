mod change_set;
mod dependency_index;
mod digest;
mod input_snapshot;
mod invalidation;
mod keys;
mod layer_cache;
mod quarantine;
#[cfg(test)]
mod run_manifest;
mod run_report;
mod stats;

#[cfg_attr(
    test,
    allow(
        unused_imports,
        reason = "Unit tests exercise only selected re-exported demand query vocabulary terms."
    )
)]
#[cfg_attr(
    not(test),
    expect(
        unused_imports,
        reason = "Demand query engine infrastructure is established before Plan 04 wires real demand-driven consumers."
    )
)]
pub(crate) use crate::analysis_neutral::demand::{
    DemandQueryEngine, DemandQueryResult, DemandQueryTrace, DemandQueryTraceEntry,
};
#[cfg_attr(
    test,
    allow(
        unused_imports,
        reason = "Unit tests exercise only selected re-exported cache vocabulary terms."
    )
)]
#[cfg_attr(
    not(test),
    expect(
        unused_imports,
        reason = "Layer manifest reuse consumes this vocabulary; some future change kinds remain reserved."
    )
)]
pub(crate) use change_set::{ChangeKind, ChangeSet, ChangeSetRow};
#[cfg_attr(
    test,
    allow(
        unused_imports,
        reason = "Unit tests exercise only selected re-exported cache vocabulary terms."
    )
)]
#[cfg_attr(
    not(test),
    expect(
        unused_imports,
        reason = "Layer manifests consume dependency indexes; some future shapes remain reserved."
    )
)]
pub(crate) use dependency_index::{
    CacheNode, DEPENDENCY_INDEX_SCHEMA, DependencyEdge, DependencyIndex, DependencyKind, ShapeKind,
};
pub(crate) use digest::{Digest, DigestBuilder, DigestKind};
#[cfg_attr(
    test,
    allow(
        unused_imports,
        reason = "Unit tests exercise only selected re-exported cache vocabulary terms."
    )
)]
#[cfg_attr(
    not(test),
    expect(unused_imports, reason = "kept for private internal consumers")
)]
pub(crate) use input_snapshot::{
    FileSnapshot, GoLifecycleSnapshot, INPUT_SNAPSHOT_SCHEMA_VERSION, InputComponent,
    InputComponentStatus, InputSnapshot, ProviderSchemaSnapshot, TsJsLifecycleSnapshot,
    input_snapshot_from_run_inputs,
};
#[cfg_attr(
    test,
    allow(
        unused_imports,
        reason = "Unit tests exercise only selected re-exported cache vocabulary terms."
    )
)]
#[cfg_attr(
    not(test),
    expect(
        unused_imports,
        reason = "Layer manifest reuse consumes invalidation plans; some future actions remain reserved."
    )
)]
pub(crate) use invalidation::{
    DropReason, InvalidationAction, InvalidationPlan, InvalidationStats, QuarantineReason,
    RecomputeReason, VerifyReason,
};
#[cfg_attr(
    test,
    allow(
        unused_imports,
        reason = "Unit tests exercise only selected re-exported cache vocabulary terms."
    )
)]
#[cfg_attr(
    not(test),
    expect(unused_imports, reason = "kept for private internal consumers")
)]
pub(crate) use keys::{
    DiagnosticKey, LayerKey, LayerKind, MODULE_GRAPH_TOPOLOGY_INPUT_FILE_NAMES, PrecisionTier,
    QueryKey, SummaryKey, dependency_layer_digest, module_graph_topology_input_digest_rows,
    module_graph_topology_input_digests, semantic_provider_parameter_digest,
};
#[cfg_attr(
    test,
    allow(
        unused_imports,
        reason = "Unit tests exercise only selected re-exported cache vocabulary terms."
    )
)]
#[cfg_attr(
    not(test),
    expect(unused_imports, reason = "kept for private internal consumers")
)]
pub(crate) use layer_cache::{
    LAYER_CACHE_MANIFEST_SCHEMA, LayerCacheManifest, LayerCacheReadOutcome, LayerCacheReadStatus,
    LayerCacheStore, LayerCacheWriteStatus, relative_manifest_dependency_source,
};
#[cfg_attr(
    test,
    allow(
        unused_imports,
        reason = "Unit tests exercise only selected re-exported quarantine vocabulary terms."
    )
)]
#[cfg_attr(
    not(test),
    expect(unused_imports, reason = "kept for private internal consumers")
)]
pub(crate) use quarantine::{QuarantineEntry, QuarantinePolicy, QuarantineStore};
#[cfg(test)]
pub(crate) use run_manifest::{
    EncodedRunManifest, EncodedRunManifestSource, RunManifest, RunManifestError, RunManifestInputs,
};
pub(crate) use run_report::{
    KernelRunReport, provider_output_digest_from_manifest, provider_output_from_manifest,
    provider_output_identity_from_manifest,
};
pub(crate) use stats::{CacheStats, ProviderOutputMeta, ProviderTelemetry};

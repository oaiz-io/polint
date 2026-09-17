use std::collections::{BTreeMap, BTreeSet};

use super::facts::{
    TypeConfidence, TypeFact, TypePhase, TypePrecision, TypeProvenance, TypeShape, TypeStatus,
    TypeSubject,
};
use super::store::TypeValueAliasOutput;
use crate::analysis_neutral::access_paths::facts::{
    AccessPathFact, AccessPathProjection, AccessPathStatus,
};
use crate::analysis_neutral::aliases::facts::{
    AliasAnswerFact, AliasOperand, AliasPrecision, AliasReason, AliasStatus,
};
use crate::analysis_neutral::extensions::sinks::{
    ExtensionFactPrecision, TYPE_VALUE_ALIAS_ACCESS_PATH_FAMILY,
    TYPE_VALUE_ALIAS_ALIAS_ANSWER_FAMILY, TYPE_VALUE_ALIAS_ALLOCATION_FAMILY,
    TYPE_VALUE_ALIAS_POINTS_TO_CONSTRAINT_FAMILY, TYPE_VALUE_ALIAS_TYPE_FAMILY,
    TYPE_VALUE_ALIAS_VALUE_FAMILY,
};
use crate::analysis_neutral::extensions::store::AcceptedExtensionFact;
use crate::analysis_neutral::ids::{
    AbstractValueId, AccessPathId, AliasAnswerId, AllocationTokenId, ObjectTokenId, PlaceId,
    PointsToConstraintId, PtVarId, TypeFactId, TypeSetId, ValueFactId,
};
use crate::analysis_neutral::points_to::facts::{
    PointsToConstraintFact, PointsToConstraintKind, PointsToPrecision, PointsToStatus,
};
use crate::analysis_neutral::values::facts::{
    AllocationKind, AllocationTokenFact, ValueFact, ValueKind, ValuePrecision, ValueProvenance,
    ValueStatus, ValueSubject,
};
use crate::internal_core::{Diagnostic, DiagnosticRange};
use crate::internal_core::{Language, StableKeyId};

pub struct ExtensionTypeValueAliasMerge {
    pub output: TypeValueAliasOutput,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn merge_extension_type_value_alias_facts(
    interner: &crate::internal_core::StableKeyInterner,
    output: TypeValueAliasOutput,
    extension_facts: &[AcceptedExtensionFact],
) -> TypeValueAliasOutput {
    let base_merge = merge_extension_type_value_alias_base_facts(interner, output, extension_facts);
    let relation_merge = merge_extension_type_value_alias_relation_facts(
        interner,
        base_merge.output,
        extension_facts,
    );
    relation_merge.output.normalized(interner)
}

pub fn merge_extension_type_value_alias_base_facts(
    interner: &crate::internal_core::StableKeyInterner,
    output: TypeValueAliasOutput,
    extension_facts: &[AcceptedExtensionFact],
) -> ExtensionTypeValueAliasMerge {
    merge_extension_type_value_alias_facts_with_filter(
        interner,
        output,
        extension_facts,
        |family| {
            matches!(
                family,
                TYPE_VALUE_ALIAS_TYPE_FAMILY
                    | TYPE_VALUE_ALIAS_VALUE_FAMILY
                    | TYPE_VALUE_ALIAS_ALLOCATION_FAMILY
                    | TYPE_VALUE_ALIAS_ACCESS_PATH_FAMILY
            )
        },
    )
}

pub fn merge_extension_type_value_alias_relation_facts(
    interner: &crate::internal_core::StableKeyInterner,
    output: TypeValueAliasOutput,
    extension_facts: &[AcceptedExtensionFact],
) -> ExtensionTypeValueAliasMerge {
    merge_extension_type_value_alias_facts_with_filter(
        interner,
        output,
        extension_facts,
        |family| {
            matches!(
                family,
                TYPE_VALUE_ALIAS_POINTS_TO_CONSTRAINT_FAMILY | TYPE_VALUE_ALIAS_ALIAS_ANSWER_FAMILY
            )
        },
    )
}

fn merge_extension_type_value_alias_facts_with_filter(
    interner: &crate::internal_core::StableKeyInterner,
    mut output: TypeValueAliasOutput,
    extension_facts: &[AcceptedExtensionFact],
    include_family: impl Fn(&str) -> bool,
) -> ExtensionTypeValueAliasMerge {
    let mut diagnostics = Vec::new();
    let mut stable_keys = native_stable_keys(&output);
    let extension_refs = ExtensionRefMaps::from_output(interner, &output, extension_facts);
    for fact in extension_facts {
        if !include_family(fact.fact_family.as_str()) {
            continue;
        }
        if stable_keys.contains(&fact.stable_key) {
            diagnostics.push(extension_merge_diagnostic(
                interner,
                fact,
                "stable_key_conflict",
            ));
            continue;
        }
        let inserted = match fact.fact_family.as_str() {
            TYPE_VALUE_ALIAS_TYPE_FAMILY => merge_type_fact(&mut output, fact),
            TYPE_VALUE_ALIAS_VALUE_FAMILY => merge_value_fact(&mut output, fact),
            TYPE_VALUE_ALIAS_ALLOCATION_FAMILY => merge_allocation_fact(&mut output, fact),
            TYPE_VALUE_ALIAS_ACCESS_PATH_FAMILY => merge_access_path_fact(&mut output, fact),
            TYPE_VALUE_ALIAS_POINTS_TO_CONSTRAINT_FAMILY => {
                merge_points_to_constraint(&mut output, fact, &extension_refs)
            }
            TYPE_VALUE_ALIAS_ALIAS_ANSWER_FAMILY => {
                merge_alias_answer(&mut output, fact, &extension_refs)
            }
            _ => false,
        };
        if inserted {
            stable_keys.insert(fact.stable_key);
        } else {
            diagnostics.push(extension_merge_diagnostic(
                interner,
                fact,
                "malformed_payload",
            ));
        }
    }
    ExtensionTypeValueAliasMerge {
        output: output.normalized(interner),
        diagnostics,
    }
}

fn native_stable_keys(output: &TypeValueAliasOutput) -> BTreeSet<StableKeyId> {
    output
        .types
        .types
        .iter()
        .map(|fact| fact.stable_key)
        .chain(output.types.narrowed.iter().map(|fact| fact.stable_key))
        .chain(output.values.values.iter().map(|fact| fact.stable_key))
        .chain(output.values.allocations.iter().map(|fact| fact.stable_key))
        .chain(
            output
                .access_paths
                .access_paths
                .iter()
                .map(|fact| fact.stable_key),
        )
        .chain(
            output
                .points_to
                .constraints
                .iter()
                .map(|fact| fact.stable_key),
        )
        .chain(output.points_to.sets.iter().map(|fact| fact.stable_key))
        .chain(output.aliases.answers.iter().map(|fact| fact.stable_key))
        .collect()
}

fn merge_type_fact(output: &mut TypeValueAliasOutput, fact: &AcceptedExtensionFact) -> bool {
    let Some(subject) = label(fact, "subject=").and_then(type_subject) else {
        return false;
    };
    let Some(shape) = label(fact, "shape=").map(type_shape) else {
        return false;
    };
    output.types.types.push(TypeFact {
        id: TypeFactId(output.types.types.len() as u64),
        subject,
        type_set: TypeSetId(output.types.types.len() as u64),
        shape,
        phase: TypePhase::ExtensionProvided,
        language: language_label(label(fact, "language=")),
        file: None,
        function: None,
        body: None,
        place: label(fact, "place=").and_then(place_id),
        cfg_block: None,
        operation: None,
        precision: type_precision(fact.precision),
        confidence: type_confidence(fact),
        status: TypeStatus::Present,
        provenance: TypeProvenance::Extension {
            extension_id: fact.extension_id.clone(),
        },
        stable_key: fact.stable_key,
    });
    true
}

fn merge_value_fact(output: &mut TypeValueAliasOutput, fact: &AcceptedExtensionFact) -> bool {
    let Some(subject) = label(fact, "subject=").and_then(value_subject) else {
        return false;
    };
    let Some(kind) = label(fact, "kind=").map(value_kind) else {
        return false;
    };
    let id = ValueFactId(output.values.values.len() as u64);
    output.values.values.push(ValueFact {
        id,
        subject,
        value: AbstractValueId(id.0),
        kind,
        language: language_label(label(fact, "language=")),
        file: None,
        function: None,
        body: None,
        precision: value_precision(fact.precision),
        status: ValueStatus::Present,
        provenance: ValueProvenance::Extension {
            extension_id: fact.extension_id.clone(),
        },
        stable_key: fact.stable_key,
    });
    true
}

fn merge_allocation_fact(output: &mut TypeValueAliasOutput, fact: &AcceptedExtensionFact) -> bool {
    let Some(kind) = label(fact, "kind=").map(allocation_kind) else {
        return false;
    };
    output.values.allocations.push(AllocationTokenFact {
        id: AllocationTokenId(output.values.allocations.len() as u64),
        kind,
        language: language_label(label(fact, "language=")),
        file: None,
        function: None,
        body: None,
        source_place: label(fact, "source_place=").and_then(place_id),
        source_operation: None,
        span: None,
        provenance: ValueProvenance::Extension {
            extension_id: fact.extension_id.clone(),
        },
        stable_key: fact.stable_key,
    });
    true
}

fn merge_access_path_fact(output: &mut TypeValueAliasOutput, fact: &AcceptedExtensionFact) -> bool {
    let Some(base) = label(fact, "base=").and_then(place_id) else {
        return false;
    };
    let projections = access_path_projections(fact);
    output.access_paths.access_paths.push(AccessPathFact {
        id: AccessPathId(output.access_paths.access_paths.len() as u64),
        base,
        depth: projections.len() as u32,
        projections,
        language: language_label(label(fact, "language=")),
        file: None,
        function: None,
        body: None,
        status: AccessPathStatus::Resolved,
        stable_key: fact.stable_key,
    });
    true
}

fn merge_points_to_constraint(
    output: &mut TypeValueAliasOutput,
    fact: &AcceptedExtensionFact,
    extension_refs: &ExtensionRefMaps,
) -> bool {
    let Some(kind) = extension_constraint_kind(fact, extension_refs) else {
        return false;
    };
    output.points_to.constraints.push(PointsToConstraintFact {
        id: PointsToConstraintId(output.points_to.constraints.len() as u64),
        kind,
        status: PointsToStatus::Present,
        precision: points_to_precision(fact.precision),
        stable_key: fact.stable_key,
    });
    true
}

fn merge_alias_answer(
    output: &mut TypeValueAliasOutput,
    fact: &AcceptedExtensionFact,
    extension_refs: &ExtensionRefMaps,
) -> bool {
    let Some(left) =
        label(fact, "left=").and_then(|value| alias_operand(fact, value, extension_refs))
    else {
        return false;
    };
    let Some(right) =
        label(fact, "right=").and_then(|value| alias_operand(fact, value, extension_refs))
    else {
        return false;
    };
    let Some(status) = label(fact, "status=").and_then(alias_status) else {
        return false;
    };
    output.aliases.answers.push(AliasAnswerFact {
        id: AliasAnswerId(output.aliases.answers.len() as u64),
        left,
        right,
        status,
        reason: AliasReason::ExtensionProvided,
        evidence: extension_evidence(fact),
        precision: alias_precision(fact.precision),
        stable_key: fact.stable_key,
    });
    true
}

fn extension_constraint_kind(
    fact: &AcceptedExtensionFact,
    extension_refs: &ExtensionRefMaps,
) -> Option<PointsToConstraintKind> {
    match label(fact, "kind=")? {
        "address_of" => Some(PointsToConstraintKind::AddressOf {
            dst: label(fact, "dst=").and_then(|value| pt_var_id(fact, value, extension_refs))?,
            object: label(fact, "object=")
                .and_then(|value| object_token_id(fact, value, extension_refs))?,
        }),
        "copy" => Some(PointsToConstraintKind::Copy {
            dst: label(fact, "dst=").and_then(|value| pt_var_id(fact, value, extension_refs))?,
            src: label(fact, "src=").and_then(|value| pt_var_id(fact, value, extension_refs))?,
        }),
        _ => None,
    }
}

#[derive(Default)]
struct ExtensionRefMaps {
    access_paths: BTreeMap<ExtensionLocalRef, AccessPathId>,
    allocations: BTreeMap<ExtensionLocalRef, AllocationTokenId>,
    values: BTreeMap<ExtensionLocalRef, ValueFactId>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ExtensionLocalRef {
    extension_id: String,
    provider_id: String,
    local_id: u64,
}

impl ExtensionRefMaps {
    fn from_output(
        interner: &crate::internal_core::StableKeyInterner,
        output: &TypeValueAliasOutput,
        extension_facts: &[AcceptedExtensionFact],
    ) -> Self {
        let access_path_ids_by_stable_key = output
            .access_paths
            .access_paths
            .iter()
            .map(|fact| (fact.stable_key, fact.id))
            .collect::<BTreeMap<_, _>>();
        let allocation_ids_by_stable_key = output
            .values
            .allocations
            .iter()
            .map(|fact| (fact.stable_key, fact.id))
            .collect::<BTreeMap<_, _>>();
        let value_ids_by_stable_key = output
            .values
            .values
            .iter()
            .map(|fact| (fact.stable_key, fact.id))
            .collect::<BTreeMap<_, _>>();

        let mut maps = Self::default();
        maps.record_family_refs(
            interner,
            extension_facts,
            TYPE_VALUE_ALIAS_ACCESS_PATH_FAMILY,
            &access_path_ids_by_stable_key,
            |maps, key, id| {
                maps.access_paths.insert(key, id);
            },
        );
        maps.record_family_refs(
            interner,
            extension_facts,
            TYPE_VALUE_ALIAS_ALLOCATION_FAMILY,
            &allocation_ids_by_stable_key,
            |maps, key, id| {
                maps.allocations.insert(key, id);
            },
        );
        maps.record_family_refs(
            interner,
            extension_facts,
            TYPE_VALUE_ALIAS_VALUE_FAMILY,
            &value_ids_by_stable_key,
            |maps, key, id| {
                maps.values.insert(key, id);
            },
        );
        maps
    }

    fn record_family_refs<T: Copy>(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        extension_facts: &[AcceptedExtensionFact],
        family: &str,
        ids_by_stable_key: &BTreeMap<StableKeyId, T>,
        mut insert: impl FnMut(&mut Self, ExtensionLocalRef, T),
    ) {
        let mut local_counts = BTreeMap::<(&str, &str), u64>::new();
        let mut facts = extension_facts
            .iter()
            .filter(|fact| fact.fact_family == family)
            .collect::<Vec<_>>();
        // Local IDs are ordinals over text-sorted stable keys so relation payloads
        // stay deterministic even when interning order differs from lexical order.
        facts.sort_by(|left, right| {
            left.extension_id
                .as_str()
                .cmp(right.extension_id.as_str())
                .then_with(|| left.provider_id.as_str().cmp(right.provider_id.as_str()))
                .then_with(|| interner.compare_canonical(left.stable_key, right.stable_key))
        });

        for fact in facts {
            let count = local_counts
                .entry((fact.extension_id.as_str(), fact.provider_id.as_str()))
                .or_default();
            if let Some(id) = ids_by_stable_key.get(&fact.stable_key).copied() {
                insert(
                    self,
                    ExtensionLocalRef {
                        extension_id: fact.extension_id.clone(),
                        provider_id: fact.provider_id.clone(),
                        local_id: *count,
                    },
                    id,
                );
            }
            *count += 1;
        }
    }

    fn access_path(&self, fact: &AcceptedExtensionFact, local_id: u64) -> Option<AccessPathId> {
        self.access_paths.get(&local_ref(fact, local_id)).copied()
    }

    fn allocation(&self, fact: &AcceptedExtensionFact, local_id: u64) -> Option<AllocationTokenId> {
        self.allocations.get(&local_ref(fact, local_id)).copied()
    }

    fn value(&self, fact: &AcceptedExtensionFact, local_id: u64) -> Option<ValueFactId> {
        self.values.get(&local_ref(fact, local_id)).copied()
    }
}

fn local_ref(fact: &AcceptedExtensionFact, local_id: u64) -> ExtensionLocalRef {
    ExtensionLocalRef {
        extension_id: fact.extension_id.clone(),
        provider_id: fact.provider_id.clone(),
        local_id,
    }
}

fn label<'a>(fact: &'a AcceptedExtensionFact, prefix: &str) -> Option<&'a str> {
    fact.payload_labels
        .iter()
        .find_map(|label| label.strip_prefix(prefix))
}

fn type_subject(value: &str) -> Option<TypeSubject> {
    if let Some(place) = value.strip_prefix("place:") {
        place.parse().ok().map(|id| TypeSubject::Place(PlaceId(id)))
    } else if let Some(symbol) = value.strip_prefix("symbol:") {
        symbol
            .parse()
            .ok()
            .map(|id| TypeSubject::Symbol(crate::internal_core::SymbolId::from_raw(id)))
    } else if let Some(id) = value.strip_prefix("synthetic:") {
        Some(TypeSubject::Synthetic(id.to_string()))
    } else {
        Some(TypeSubject::Unknown(value.to_string()))
    }
}

fn value_subject(value: &str) -> Option<ValueSubject> {
    if let Some(place) = value.strip_prefix("place:") {
        place
            .parse()
            .ok()
            .map(|id| ValueSubject::Place(PlaceId(id)))
    } else if let Some(id) = value.strip_prefix("synthetic:") {
        Some(ValueSubject::Synthetic(id.to_string()))
    } else {
        Some(ValueSubject::Unknown(value.to_string()))
    }
}

fn type_shape(value: &str) -> TypeShape {
    match value {
        "any" => TypeShape::Any,
        "object" => TypeShape::Object { shape_id: None },
        "class" => TypeShape::Class { name: None },
        other => {
            if let Some(primitive) = other.strip_prefix("primitive:") {
                TypeShape::Primitive(primitive.to_string())
            } else if let Some(literal) = other.strip_prefix("literal:") {
                TypeShape::Literal(literal.to_string())
            } else if let Some(signature) = other.strip_prefix("callable:") {
                TypeShape::Callable {
                    signature: signature.to_string(),
                }
            } else if let Some(module_key) = other.strip_prefix("module:") {
                TypeShape::Module {
                    module_key: module_key.to_string(),
                }
            } else if let Some(type_id) = other.strip_prefix("nominal:") {
                TypeShape::Nominal {
                    type_id: type_id.to_string(),
                }
            } else if let Some(shape_id) = other.strip_prefix("structural:") {
                TypeShape::Structural {
                    shape_id: shape_id.to_string(),
                }
            } else if let Some(reason) = other.strip_prefix("unsupported:") {
                TypeShape::Unsupported {
                    reason: reason.to_string(),
                }
            } else {
                TypeShape::Unknown {
                    reason: other.to_string(),
                }
            }
        }
    }
}

fn value_kind(value: &str) -> ValueKind {
    match value {
        "null" => ValueKind::Null,
        "undefined" => ValueKind::Undefined,
        "nil" => ValueKind::Nil,
        "function" => ValueKind::FunctionObject,
        "class" => ValueKind::ClassObject,
        "module" => ValueKind::ModuleObject,
        other => {
            if let Some(value) = other.strip_prefix("bool:") {
                ValueKind::Bool(value.to_string())
            } else if let Some(value) = other.strip_prefix("number:") {
                ValueKind::Number(value.to_string())
            } else if let Some(value) = other.strip_prefix("string:") {
                ValueKind::String(value.to_string())
            } else if let Some(value) = other.strip_prefix("literal:") {
                ValueKind::Literal(value.to_string())
            } else if let Some(place) = other.strip_prefix("place_ref:") {
                place
                    .parse()
                    .map(|id| ValueKind::PlaceRef(PlaceId(id)))
                    .unwrap_or_else(|_| ValueKind::Unknown {
                        evidence: other.to_string(),
                    })
            } else {
                ValueKind::Unknown {
                    evidence: other.to_string(),
                }
            }
        }
    }
}

fn allocation_kind(value: &str) -> AllocationKind {
    match value {
        "object" => AllocationKind::ObjectLiteral,
        "array" => AllocationKind::ArrayLiteral,
        "composite" => AllocationKind::CompositeLiteral,
        "function" => AllocationKind::FunctionObject,
        "class" => AllocationKind::ClassObject,
        "module" => AllocationKind::ModuleNamespace,
        "closure" => AllocationKind::Closure,
        "synthetic_framework" => AllocationKind::SyntheticFrameworkObject,
        "extension" => AllocationKind::ExtensionModeledObject,
        _ => AllocationKind::Unknown,
    }
}

fn access_path_projections(fact: &AcceptedExtensionFact) -> Vec<AccessPathProjection> {
    let has_base_projection = fact
        .payload_labels
        .iter()
        .any(|label| label == "projection=base");
    let projections = fact
        .payload_labels
        .iter()
        .filter_map(|label| label.strip_prefix("projection="))
        .filter(|value| *value != "base")
        .map(|value| {
            if let Some(field) = value.strip_prefix("field:") {
                AccessPathProjection::Field(field.to_string())
            } else if let Some(property) = value.strip_prefix("property:") {
                AccessPathProjection::Property(property.to_string())
            } else if let Some(index) = value.strip_prefix("index:") {
                AccessPathProjection::IndexKnown(index.to_string())
            } else if value == "deref" {
                AccessPathProjection::Deref
            } else if value == "await" {
                AccessPathProjection::AwaitResult
            } else {
                AccessPathProjection::Unknown {
                    evidence: value.to_string(),
                }
            }
        })
        .collect::<Vec<_>>();
    if projections.is_empty() {
        if has_base_projection {
            Vec::new()
        } else {
            vec![AccessPathProjection::Unknown {
                evidence: "extension-base-only".to_string(),
            }]
        }
    } else {
        projections
    }
}

fn alias_operand(
    fact: &AcceptedExtensionFact,
    value: &str,
    extension_refs: &ExtensionRefMaps,
) -> Option<AliasOperand> {
    if let Some(place) = value.strip_prefix("place:") {
        place
            .parse()
            .ok()
            .map(|id| AliasOperand::Place(PlaceId(id)))
    } else if let Some(path) = value.strip_prefix("access_path:") {
        path.parse()
            .ok()
            .and_then(|id| extension_refs.access_path(fact, id))
            .map(AliasOperand::AccessPath)
    } else {
        None
    }
}

fn alias_status(value: &str) -> Option<AliasStatus> {
    match value {
        "no_alias" => Some(AliasStatus::NoAlias),
        "may_alias" => Some(AliasStatus::MayAlias),
        "must_alias" => Some(AliasStatus::MustAlias),
        "partial_alias" => Some(AliasStatus::PartialAlias),
        "unknown" => Some(AliasStatus::Unknown),
        _ => None,
    }
}

fn place_id(value: &str) -> Option<PlaceId> {
    value
        .strip_prefix("place:")
        .unwrap_or(value)
        .parse()
        .ok()
        .map(PlaceId)
}

fn pt_var_id(
    fact: &AcceptedExtensionFact,
    value: &str,
    extension_refs: &ExtensionRefMaps,
) -> Option<PtVarId> {
    if let Some(place) = value.strip_prefix("place:") {
        place
            .parse()
            .ok()
            .map(PlaceId)
            .map(crate::analysis_neutral::points_to::vars::place_var)
    } else if let Some(path) = value.strip_prefix("access_path:") {
        path.parse()
            .ok()
            .and_then(|id| extension_refs.access_path(fact, id))
            .map(crate::analysis_neutral::points_to::vars::access_path_var)
    } else if let Some(allocation) = value.strip_prefix("allocation:") {
        allocation
            .parse()
            .ok()
            .and_then(|id| extension_refs.allocation(fact, id))
            .map(crate::analysis_neutral::points_to::vars::allocation_var)
    } else {
        None
    }
}

fn object_token_id(
    fact: &AcceptedExtensionFact,
    value: &str,
    extension_refs: &ExtensionRefMaps,
) -> Option<ObjectTokenId> {
    if let Some(allocation) = value.strip_prefix("allocation:") {
        allocation
            .parse()
            .ok()
            .and_then(|id| extension_refs.allocation(fact, id))
            .map(crate::analysis_neutral::points_to::vars::allocation_object)
    } else if let Some(value_fact) = value.strip_prefix("value:") {
        value_fact
            .parse()
            .ok()
            .and_then(|id| extension_refs.value(fact, id))
            .map(crate::analysis_neutral::points_to::vars::value_fact_object)
    } else if let Some(abstract_value) = value.strip_prefix("abstract_value:") {
        abstract_value
            .parse()
            .ok()
            .and_then(|id| {
                extension_refs
                    .value(fact, id)
                    .map(|value| AbstractValueId(value.0))
            })
            .map(crate::analysis_neutral::points_to::vars::abstract_value_object)
    } else {
        None
    }
}

fn language_label(value: Option<&str>) -> Language {
    match value {
        Some("go") => Language::Go,
        Some("javascript") => Language::JavaScript,
        Some("jsx") => Language::Jsx,
        Some("typescript") => Language::TypeScript,
        Some("tsx") => Language::Tsx,
        _ => Language::Unknown,
    }
}

fn type_precision(precision: ExtensionFactPrecision) -> TypePrecision {
    match precision {
        ExtensionFactPrecision::Exact => TypePrecision::ExactLocal,
        ExtensionFactPrecision::SetupAware => TypePrecision::SetupAware,
        ExtensionFactPrecision::Heuristic | ExtensionFactPrecision::GeneratedUnvalidated => {
            TypePrecision::Heuristic
        }
    }
}

fn type_confidence(fact: &AcceptedExtensionFact) -> TypeConfidence {
    match fact.confidence {
        crate::analysis_neutral::extensions::sinks::ExtensionFactConfidence::High => {
            TypeConfidence::High
        }
        crate::analysis_neutral::extensions::sinks::ExtensionFactConfidence::Medium => {
            TypeConfidence::Medium
        }
        crate::analysis_neutral::extensions::sinks::ExtensionFactConfidence::Low => {
            TypeConfidence::Low
        }
    }
}

fn value_precision(precision: ExtensionFactPrecision) -> ValuePrecision {
    match precision {
        ExtensionFactPrecision::Exact => ValuePrecision::ExactLocal,
        ExtensionFactPrecision::SetupAware => ValuePrecision::SetupAware,
        ExtensionFactPrecision::Heuristic | ExtensionFactPrecision::GeneratedUnvalidated => {
            ValuePrecision::Heuristic
        }
    }
}

fn points_to_precision(precision: ExtensionFactPrecision) -> PointsToPrecision {
    match precision {
        ExtensionFactPrecision::Exact => PointsToPrecision::LocalFlowSensitive,
        ExtensionFactPrecision::SetupAware => PointsToPrecision::SummaryProjected,
        ExtensionFactPrecision::Heuristic | ExtensionFactPrecision::GeneratedUnvalidated => {
            PointsToPrecision::Heuristic
        }
    }
}

fn alias_precision(precision: ExtensionFactPrecision) -> AliasPrecision {
    match precision {
        ExtensionFactPrecision::Exact => AliasPrecision::ExactLocal,
        ExtensionFactPrecision::SetupAware => AliasPrecision::SetupAware,
        ExtensionFactPrecision::Heuristic | ExtensionFactPrecision::GeneratedUnvalidated => {
            AliasPrecision::Heuristic
        }
    }
}

fn extension_evidence(fact: &AcceptedExtensionFact) -> Vec<String> {
    let mut evidence = vec![format!(
        "extension:{} provider:{}",
        fact.extension_id, fact.provider_id
    )];
    evidence.extend(fact.evidence.iter().cloned());
    evidence.sort();
    evidence.dedup();
    evidence
}

fn extension_merge_diagnostic(
    interner: &crate::internal_core::StableKeyInterner,
    fact: &AcceptedExtensionFact,
    reason: &'static str,
) -> Diagnostic {
    Diagnostic::error(
        "polint/extension",
        "<workspace>",
        DiagnosticRange::point(1, 1),
        "Accepted extension type/value/alias fact could not be merged.",
    )
    .with_evidence("extension_id", fact.extension_id.clone())
    .with_evidence("provider_id", fact.provider_id.clone())
    .with_evidence("fact_family", fact.fact_family.clone())
    .with_evidence("stable_key", interner.resolve(fact.stable_key).to_string())
    .with_evidence("reason", reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::aliases::facts::{AliasAnswerFact, AliasReason};
    use crate::analysis_neutral::aliases::store::AliasOutput;
    use crate::analysis_neutral::extensions::sinks::{
        ExtensionFactConfidence, ExtensionFactPrecision, ExtensionFactStatus,
        TYPE_VALUE_ALIAS_ACCESS_PATH_FAMILY, TYPE_VALUE_ALIAS_ALLOCATION_FAMILY,
        TYPE_VALUE_ALIAS_POINTS_TO_CONSTRAINT_FAMILY, TYPE_VALUE_ALIAS_TYPE_FAMILY,
        TYPE_VALUE_ALIAS_VALUE_FAMILY,
    };

    fn test_interner() -> crate::internal_core::StableKeyInterner {
        crate::analysis_neutral::LocalAnalysisDb::new().stable_key_interner()
    }

    #[test]
    fn extension_merge_adds_type_value_allocation_access_path_and_points_to_rows() {
        let interner = test_interner();
        let merged = merge_extension_type_value_alias_facts(
            &interner,
            TypeValueAliasOutput::default(),
            &[
                accepted(
                    TYPE_VALUE_ALIAS_TYPE_FAMILY,
                    "type:extension",
                    &["subject=synthetic:t", "shape=primitive:string"],
                ),
                accepted(
                    TYPE_VALUE_ALIAS_VALUE_FAMILY,
                    "value:extension",
                    &["subject=synthetic:v", "kind=string:ok"],
                ),
                accepted(
                    TYPE_VALUE_ALIAS_ALLOCATION_FAMILY,
                    "alloc:extension",
                    &["kind=object"],
                ),
                accepted(
                    TYPE_VALUE_ALIAS_ACCESS_PATH_FAMILY,
                    "path:extension",
                    &["base=place:1", "projection=property:name"],
                ),
                accepted(
                    TYPE_VALUE_ALIAS_POINTS_TO_CONSTRAINT_FAMILY,
                    "pt:extension",
                    &["kind=copy", "dst=place:1", "src=place:2"],
                ),
            ],
        );

        assert_eq!(merged.types.types.len(), 1);
        assert_eq!(merged.values.values.len(), 1);
        assert_eq!(merged.values.allocations.len(), 1);
        assert_eq!(merged.access_paths.access_paths.len(), 1);
        assert_eq!(merged.points_to.constraints.len(), 1);
    }

    #[test]
    fn extension_merge_is_additive_and_preserves_native_unknown_alias() {
        let output = TypeValueAliasOutput {
            aliases: AliasOutput {
                answers: vec![AliasAnswerFact {
                    id: AliasAnswerId(0),
                    left: AliasOperand::Place(PlaceId(1)),
                    right: AliasOperand::Place(PlaceId(2)),
                    status: AliasStatus::Unknown,
                    reason: AliasReason::MissingPointsTo,
                    evidence: vec!["native-missing".to_string()],
                    precision: AliasPrecision::Unknown,
                    stable_key: crate::internal_core::stable_key_for_test("alias:native:unknown"),
                }],
            },
            ..TypeValueAliasOutput::default()
        };

        let interner = test_interner();
        let merged = merge_extension_type_value_alias_facts(
            &interner,
            output,
            &[accepted_alias("alias:extension:no", "no_alias")],
        );

        assert!(
            merged
                .aliases
                .answers
                .iter()
                .any(|answer| answer.status == AliasStatus::Unknown)
        );
        assert!(
            merged
                .aliases
                .answers
                .iter()
                .any(|answer| answer.status == AliasStatus::NoAlias
                    && answer.reason == AliasReason::ExtensionProvided)
        );
    }

    #[test]
    fn extension_merge_skips_conflicting_stable_key() {
        let output = TypeValueAliasOutput {
            aliases: AliasOutput {
                answers: vec![AliasAnswerFact {
                    id: AliasAnswerId(0),
                    left: AliasOperand::Place(PlaceId(1)),
                    right: AliasOperand::Place(PlaceId(2)),
                    status: AliasStatus::Unknown,
                    reason: AliasReason::MissingPointsTo,
                    evidence: vec!["native-missing".to_string()],
                    precision: AliasPrecision::Unknown,
                    stable_key: crate::internal_core::stable_key_for_test("alias:shared"),
                }],
            },
            ..TypeValueAliasOutput::default()
        };

        let interner = test_interner();
        let merged = merge_extension_type_value_alias_facts(
            &interner,
            output,
            &[accepted_alias("alias:shared", "no_alias")],
        );

        assert_eq!(merged.aliases.answers.len(), 1);
        assert_eq!(merged.aliases.answers[0].status, AliasStatus::Unknown);
    }

    #[test]
    fn extension_relation_refs_resolve_to_extension_local_base_facts() {
        let output = TypeValueAliasOutput {
            values: crate::analysis_neutral::values::store::ValueOutput {
                allocations: vec![AllocationTokenFact {
                    id: AllocationTokenId(0),
                    kind: AllocationKind::ObjectLiteral,
                    language: Language::TypeScript,
                    file: None,
                    function: None,
                    body: None,
                    source_place: None,
                    source_operation: None,
                    span: None,
                    provenance: ValueProvenance::Native,
                    stable_key: crate::internal_core::stable_key_for_test("allocation:a-native"),
                }],
                ..Default::default()
            },
            ..TypeValueAliasOutput::default()
        };

        let interner = test_interner();
        let merged = merge_extension_type_value_alias_facts(
            &interner,
            output,
            &[
                accepted(
                    TYPE_VALUE_ALIAS_ALLOCATION_FAMILY,
                    "allocation:z-extension",
                    &["kind=object"],
                ),
                accepted(
                    TYPE_VALUE_ALIAS_POINTS_TO_CONSTRAINT_FAMILY,
                    "pt:extension",
                    &["kind=address_of", "dst=allocation:0", "object=allocation:0"],
                ),
            ],
        );
        let extension_allocation = merged
            .values
            .allocations
            .iter()
            .find(|allocation| {
                interner.resolve(allocation.stable_key).as_ref() == "allocation:z-extension"
            })
            .expect("extension allocation should be present")
            .id;

        assert!(merged.points_to.constraints.iter().any(|constraint| {
            matches!(
                constraint.kind,
                PointsToConstraintKind::AddressOf { dst, object }
                    if dst == crate::analysis_neutral::points_to::vars::allocation_var(extension_allocation)
                        && object
                            == crate::analysis_neutral::points_to::vars::allocation_object(
                                extension_allocation
                            )
            )
        }));
    }

    #[test]
    fn extension_local_ids_follow_resolved_text_order_not_allocation_order() {
        let interner = crate::internal_core::StableKeyInterner::default();
        // Intern reverse-lexically so StableKeyId Ord disagrees with text Ord.
        let key_z = interner.intern("allocation:z-extension".to_string());
        let key_a = interner.intern("allocation:a-extension".to_string());
        let key_pt = interner.intern("pt:extension".to_string());
        assert!(
            key_z.0 < key_a.0,
            "precondition: allocation order must put z before a"
        );
        assert!(
            interner.resolve(key_a).as_ref() < interner.resolve(key_z).as_ref(),
            "precondition: resolved text order must put a before z"
        );

        let mut fact_z = accepted(
            TYPE_VALUE_ALIAS_ALLOCATION_FAMILY,
            "allocation:z-extension",
            &["kind=object"],
        );
        fact_z.stable_key = key_z;
        let mut fact_a = accepted(
            TYPE_VALUE_ALIAS_ALLOCATION_FAMILY,
            "allocation:a-extension",
            &["kind=object"],
        );
        fact_a.stable_key = key_a;
        let mut constraint = accepted(
            TYPE_VALUE_ALIAS_POINTS_TO_CONSTRAINT_FAMILY,
            "pt:extension",
            &["kind=address_of", "dst=allocation:0", "object=allocation:0"],
        );
        constraint.stable_key = key_pt;

        let merged = merge_extension_type_value_alias_facts(
            &interner,
            TypeValueAliasOutput::default(),
            &[fact_z, fact_a, constraint],
        );
        let lexical_first = merged
            .values
            .allocations
            .iter()
            .find(|allocation| {
                interner.resolve(allocation.stable_key).as_ref() == "allocation:a-extension"
            })
            .expect("lexically-first allocation should be present")
            .id;

        assert!(
            merged.points_to.constraints.iter().any(|constraint| {
                matches!(
                    constraint.kind,
                    PointsToConstraintKind::AddressOf { dst, object }
                        if dst == crate::analysis_neutral::points_to::vars::allocation_var(lexical_first)
                            && object
                                == crate::analysis_neutral::points_to::vars::allocation_object(
                                    lexical_first
                                )
                )
            }),
            "allocation:0 must resolve to text-first allocation:a-extension, not allocation-order first"
        );
    }

    fn accepted_alias(stable_key: &str, status: &str) -> AcceptedExtensionFact {
        let mut fact = accepted(
            TYPE_VALUE_ALIAS_ALIAS_ANSWER_FAMILY,
            stable_key,
            &["left=place:1", "right=place:2"],
        );
        fact.payload_labels.push(format!("status={status}"));
        fact
    }

    fn accepted(family: &str, stable_key: &str, payload_labels: &[&str]) -> AcceptedExtensionFact {
        AcceptedExtensionFact {
            extension_id: "demo".to_string(),
            provider_id: "aliases".to_string(),
            fact_family: family.to_string(),
            stable_key: crate::internal_core::stable_key_for_test(stable_key),
            binding_refs: vec!["file:src/app.ts".to_string()],
            precision: ExtensionFactPrecision::Heuristic,
            confidence: ExtensionFactConfidence::Medium,
            status: ExtensionFactStatus::Accepted,
            evidence: vec!["reviewed".to_string()],
            payload_labels: payload_labels
                .iter()
                .map(|label| (*label).to_string())
                .collect(),
            payload_digest: stable_key.to_string(),
        }
    }
}

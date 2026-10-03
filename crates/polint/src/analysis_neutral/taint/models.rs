//! Data-flow models as data: sources, sinks, sanitizers, opaque functions and
//! propagators, read from TOML tables (the built-in defaults and a repository's
//! own).

use serde::Deserialize;

use crate::analysis_neutral::taint::solver::{
    Arguments, ExternalModels, Matcher, SinkSpec, SourceSpec,
};

/// Names functions: by qualified name (`os/exec.Command`), or as methods of
/// named types (`receivers` with `methods`; a pointer receiver, an interface
/// method and a generic instance all match).
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
pub(crate) struct Target {
    #[serde(default)]
    pub(crate) function: Option<String>,
    #[serde(default)]
    pub(crate) functions: Vec<String>,
    #[serde(default)]
    pub(crate) receivers: Vec<String>,
    #[serde(default)]
    pub(crate) methods: Vec<String>,
}

impl Target {
    pub(crate) fn matches(&self, qualified: &str) -> bool {
        if let Some((receiver, method)) = split_method(qualified) {
            return self.methods.iter().any(|wanted| wanted == method)
                && self.receivers.iter().any(|wanted| wanted == &receiver);
        }
        let name = strip_type_args(qualified);
        self.function.as_deref() == Some(name.as_str())
            || self.functions.iter().any(|wanted| wanted == &name)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.function.is_none() && self.functions.is_empty() && self.methods.is_empty()
    }
}

/// `(*pkg/path.Type[T]).Method` → (`pkg/path.Type`, `Method`).
pub(crate) fn split_method(qualified: &str) -> Option<(String, &str)> {
    let rest = qualified.strip_prefix('(')?;
    let close = rest.find(").")?;
    let receiver = rest[..close].trim_start_matches('*');
    let method = &rest[close + 2..];
    Some((strip_type_args(receiver), method))
}

/// A name without its generic type arguments (`pkg.F[int]` → `pkg.F`).
pub(crate) fn strip_type_args(name: &str) -> String {
    match name.find('[') {
        Some(open) => name[..open].to_string(),
        None => name.to_string(),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub(crate) struct SourceModel {
    pub(crate) kind: String,
    #[serde(flatten)]
    pub(crate) target: Target,
    /// `result`, or `argument` (what argument `argument` points to).
    #[serde(default)]
    pub(crate) output: Option<String>,
    #[serde(default)]
    pub(crate) argument: Option<u16>,
    /// A parameter of this type is a source.
    #[serde(default)]
    pub(crate) parameter_type: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub(crate) struct SinkModel {
    pub(crate) kind: String,
    #[serde(flatten)]
    pub(crate) target: Target,
    /// Argument positions that are sinks (receiver excluded); none means all.
    #[serde(default)]
    pub(crate) arguments: Vec<u16>,
    /// Every argument from this position on is a sink.
    #[serde(default)]
    pub(crate) arguments_from: Option<u16>,
}

/// Where a propagator takes taint from: an argument position, or the receiver.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub(crate) enum PropagatorFrom {
    Argument(u16),
    /// `"receiver"`.
    Named(String),
}

impl PropagatorFrom {
    /// The argument position, or `None` for the receiver.
    pub(crate) fn argument(&self) -> Option<u16> {
        match self {
            PropagatorFrom::Argument(position) => Some(*position),
            PropagatorFrom::Named(_) => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub(crate) struct PropagatorModel {
    #[serde(flatten)]
    pub(crate) target: Target,
    pub(crate) from: PropagatorFrom,
    /// The argument whose pointee gets the taint.
    pub(crate) to: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub(crate) struct Models {
    #[serde(default, rename = "source")]
    pub(crate) sources: Vec<SourceModel>,
    #[serde(default, rename = "sink")]
    pub(crate) sinks: Vec<SinkModel>,
    #[serde(default, rename = "sanitizer")]
    pub(crate) sanitizers: Vec<Target>,
    #[serde(default, rename = "opaque")]
    pub(crate) opaque: Vec<Target>,
    #[serde(default, rename = "propagator")]
    pub(crate) propagators: Vec<PropagatorModel>,
}

impl Models {
    pub(crate) fn parse(text: &str) -> Result<Models, toml::de::Error> {
        toml::from_str(text)
    }

    /// Adds another model document's tables after this one's.
    pub(crate) fn extend(&mut self, other: Models) {
        self.sources.extend(other.sources);
        self.sinks.extend(other.sinks);
        self.sanitizers.extend(other.sanitizers);
        self.opaque.extend(other.opaque);
        self.propagators.extend(other.propagators);
    }

    /// The sources of one kind (`http_request`, `message_payload`), as the
    /// solver's sources.
    pub(crate) fn sources_of_kind(&self, kind: &str) -> Vec<SourceSpec> {
        self.sources
            .iter()
            .filter(|model| model.kind == kind)
            .map(|model| {
                if let Some(type_name) = &model.parameter_type {
                    SourceSpec::Parameter {
                        function: None,
                        index: None,
                        type_name: Some(type_name.clone()),
                    }
                } else if model.output.as_deref() == Some("argument") {
                    SourceSpec::CallArgumentPointee {
                        callee: Matcher::Target(model.target.clone()),
                        argument: model.argument.unwrap_or(0),
                    }
                } else {
                    SourceSpec::CallResult {
                        callee: Matcher::Target(model.target.clone()),
                        result: None,
                    }
                }
            })
            .collect()
    }

    /// The sinks of one kind (`sql`, `exec`, `log`, `publish`), as the solver's
    /// sinks.
    pub(crate) fn sinks_of_kind(&self, kind: &str) -> Vec<SinkSpec> {
        self.sinks
            .iter()
            .filter(|model| model.kind == kind)
            .map(|model| SinkSpec::CallArgument {
                callee: Matcher::Target(model.target.clone()),
                arguments: if !model.arguments.is_empty() {
                    Arguments::Only(model.arguments.clone())
                } else if let Some(first) = model.arguments_from {
                    Arguments::From(first)
                } else {
                    Arguments::All
                },
                receiver: false,
            })
            .collect()
    }

    /// What the models say library functions do with taint.
    pub(crate) fn external(&self) -> ExternalModels {
        let target = |target: &Target| Matcher::Target(target.clone());
        ExternalModels {
            sanitizers: self.sanitizers.iter().map(target).collect(),
            opaque: self.opaque.iter().map(target).collect(),
            into_argument: self
                .propagators
                .iter()
                .map(|model| (target(&model.target), model.from.argument(), model.to))
                .collect(),
        }
    }
}

//! The Go semantic sidecar's flow programs: one `flow_body` row per function
//! body of the program, lowered into the language-neutral flow program the taint
//! solver reads. Rows are lowered as they are decoded, so a large program never
//! holds its rows and its flow program at once.

use std::sync::OnceLock;

use serde::Deserialize;
use sha2::{Digest as _, Sha256};

use crate::analysis_neutral::taint::index::ProgramIndex;
use crate::analysis_neutral::taint::ir::{
    Algorithm, CallMode, Callee, FlowFunction, FlowProgram, Operand, Pos, Slot, Step, Stmt,
    StmtKind,
};

/// A program's flow bodies, with the digest of the rows they were lowered from.
pub(crate) struct GoFlowProgram {
    program: FlowProgram,
    digest: String,
    index: OnceLock<ProgramIndex>,
}

impl GoFlowProgram {
    pub(crate) fn program(&self) -> &FlowProgram {
        &self.program
    }

    /// The digest of the `flow_body` rows, in the order the sidecar wrote them.
    pub(crate) fn digest(&self) -> &str {
        &self.digest
    }

    /// The solver's indexes over the program, built on first use and shared by
    /// every query of the run.
    pub(crate) fn index(&self) -> &ProgramIndex {
        self.index.get_or_init(|| ProgramIndex::new(&self.program))
    }
}

impl PartialEq for GoFlowProgram {
    fn eq(&self, other: &Self) -> bool {
        self.digest == other.digest
    }
}

impl Eq for GoFlowProgram {}

impl std::fmt::Debug for GoFlowProgram {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GoFlowProgram")
            .field("bodies", &self.program.functions.len())
            .field("digest", &self.digest)
            .finish()
    }
}

/// Lowers `flow_body` rows as the sidecar output is decoded.
#[derive(Default)]
pub(crate) struct GoFlowProgramBuilder {
    program: FlowProgram,
    hasher: Option<Sha256>,
}

impl GoFlowProgramBuilder {
    /// Adds one decoded `flow_body` row, whose NDJSON text is `line`. A row
    /// without a body, or one whose statements or slots do not hold together,
    /// adds no body but still counts toward the digest.
    pub(crate) fn add(&mut self, row: &GoFlowRowFrame, line: &str) {
        self.hasher
            .get_or_insert_with(Sha256::new)
            .update(line.as_bytes());
        lower_flow_body(&mut self.program, row);
    }

    /// The program, or none when no `flow_body` row was decoded.
    pub(crate) fn finish(self) -> Option<GoFlowProgram> {
        let hasher = self.hasher?;
        let mut program = self.program;
        program.link();
        let digest = hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Some(GoFlowProgram {
            program,
            digest,
            index: OnceLock::new(),
        })
    }
}

fn null_as_empty<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

/// A `flow_body` row.
#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct GoFlowRowFrame {
    #[serde(default)]
    pub(crate) schema: String,
    #[serde(default)]
    pub(crate) kind: String,
    #[serde(default)]
    function: String,
    #[serde(default)]
    package_path: String,
    #[serde(default)]
    file: String,
    #[serde(default)]
    flow: Option<GoFlowBodyFrame>,
}

/// A function body's flow program, as the sidecar writes it (field names are
/// documented with the sidecar's `flowBody`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
struct GoFlowBodyFrame {
    #[serde(default, deserialize_with = "null_as_empty")]
    params: Vec<u32>,
    #[serde(default, deserialize_with = "null_as_empty")]
    free: Vec<u32>,
    #[serde(default)]
    results: u16,
    #[serde(default)]
    slots: u32,
    #[serde(default, deserialize_with = "null_as_empty")]
    names: Vec<GoFlowNameFrame>,
    #[serde(default, deserialize_with = "null_as_empty")]
    ptypes: Vec<String>,
    #[serde(default, deserialize_with = "null_as_empty")]
    globals: Vec<GoFlowNameFrame>,
    #[serde(default, deserialize_with = "null_as_empty")]
    consts: Vec<String>,
    #[serde(default)]
    classes: String,
    #[serde(default, deserialize_with = "null_as_empty")]
    stmts: Vec<GoFlowStmtFrame>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
struct GoFlowNameFrame {
    #[serde(default)]
    s: u32,
    #[serde(default)]
    n: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
struct GoFlowStmtFrame {
    #[serde(default)]
    o: String,
    #[serde(default)]
    d: u32,
    #[serde(default)]
    a: i64,
    #[serde(default)]
    b: i64,
    #[serde(default)]
    s: String,
    #[serde(default)]
    p: String,
    #[serde(default)]
    f: String,
    #[serde(default, deserialize_with = "null_as_empty")]
    args: Vec<i64>,
    #[serde(default, deserialize_with = "null_as_empty")]
    callees: Vec<String>,
    #[serde(default)]
    iface: String,
    #[serde(default)]
    alg: String,
    #[serde(default)]
    builtin: String,
    #[serde(default)]
    mode: String,
    #[serde(default)]
    fv: u32,
    #[serde(default)]
    l: u32,
    #[serde(default)]
    c: u32,
    #[serde(default)]
    sb: u32,
    #[serde(default)]
    eb: u32,
}

/// Adds one `flow_body` row's body to `program`. A row without a body, or one
/// whose statements do not parse or whose slots fall outside the body, adds
/// nothing.
fn lower_flow_body(program: &mut FlowProgram, row: &GoFlowRowFrame) {
    let Some(body) = row.flow.as_ref() else {
        return;
    };
    if row.function.is_empty() {
        return;
    }
    let Some(stmts) = body
        .stmts
        .iter()
        .map(|stmt| lower_stmt(program, stmt))
        .collect::<Option<Vec<_>>>()
    else {
        return;
    };
    let function = FlowFunction {
        name: row.function.clone(),
        unit: row.package_path.clone(),
        file: row.file.clone(),
        params: body.params.clone(),
        param_types: body.ptypes.clone(),
        free: body.free.clone(),
        results: body.results,
        slots: body.slots,
        names: body
            .names
            .iter()
            .map(|name| (name.s, name.n.clone()))
            .collect(),
        globals: body
            .globals
            .iter()
            .map(|global| (global.s, global.n.clone()))
            .collect(),
        consts: body.consts.clone(),
        classes: body.classes.as_bytes().to_vec(),
        stmts,
    };
    if valid(&function, body) {
        program.add(function);
    }
}

/// Every slot a body names is below its slot count, and its classes, when
/// present, cover every slot.
fn valid(function: &FlowFunction, body: &GoFlowBodyFrame) -> bool {
    let in_range = |slot: Slot| slot < function.slots;
    let operand = |operand: &Operand| match operand {
        Operand::Slot(slot) => in_range(*slot),
        Operand::Const(index) => (*index as usize) < function.consts.len(),
    };
    (function.classes.is_empty() || function.classes.len() == function.slots as usize)
        && function.params.iter().all(|slot| in_range(*slot))
        && function.free.iter().all(|slot| in_range(*slot))
        && body.names.iter().all(|name| in_range(name.s))
        && body.globals.iter().all(|global| in_range(global.s))
        && function.stmts.iter().all(|stmt| match &stmt.kind {
            StmtKind::Copy { dst, src } => in_range(*dst) && operand(src),
            StmtKind::Load { dst, base, .. } | StmtKind::Alias { dst, base, .. } => {
                in_range(*dst) && operand(base)
            }
            StmtKind::Store { target, value, .. } => operand(target) && operand(value),
            StmtKind::Alloc { dst } => in_range(*dst),
            StmtKind::Closure { dst, bindings, .. } => {
                in_range(*dst) && bindings.iter().all(operand)
            }
            StmtKind::Call {
                dst, closure, args, ..
            } => {
                dst.is_none_or(in_range) && closure.is_none_or(in_range) && args.iter().all(operand)
            }
            StmtKind::Return { values } => values.iter().all(operand),
        })
}

/// An operand: a slot, or `-1 - k` for constant `k`.
fn operand(raw: i64) -> Option<Operand> {
    if raw >= 0 {
        u32::try_from(raw).ok().map(Operand::Slot)
    } else {
        u32::try_from(-1 - raw).ok().map(Operand::Const)
    }
}

/// A destination or closure value, encoded as slot + 1 with 0 for none.
fn slot_ref(raw: u32) -> Option<Slot> {
    raw.checked_sub(1)
}

fn step(program: &mut FlowProgram, text: &str) -> Option<Step> {
    match text {
        "*" => Some(Step::Deref),
        "[]" => Some(Step::Elem),
        _ => {
            if let Some(name) = text.strip_prefix("f:") {
                Some(Step::Field(program.symbol(name)))
            } else if let Some(index) = text.strip_prefix("r:") {
                index.parse().ok().map(Step::Result)
            } else {
                None
            }
        }
    }
}

/// A dotted path (`*.f:Name`, `*.[]`, `[]`).
fn path(program: &mut FlowProgram, text: &str) -> Option<Vec<Step>> {
    text.split('.')
        .filter(|part| !part.is_empty())
        .map(|part| step(program, part))
        .collect()
}

fn lower_stmt(program: &mut FlowProgram, raw: &GoFlowStmtFrame) -> Option<Stmt> {
    let pos = Pos {
        line: raw.l,
        col: raw.c,
        start_byte: raw.sb,
        end_byte: raw.eb,
    };
    let kind = match raw.o.as_str() {
        "copy" => StmtKind::Copy {
            dst: slot_ref(raw.d)?,
            src: operand(raw.a)?,
        },
        "load" => StmtKind::Load {
            dst: slot_ref(raw.d)?,
            base: operand(raw.a)?,
            step: step(program, &raw.s)?,
        },
        "addr" => StmtKind::Alias {
            dst: slot_ref(raw.d)?,
            dst_path: vec![Step::Deref],
            base: operand(raw.a)?,
            path: path(program, &raw.s)?,
        },
        "alias" => StmtKind::Alias {
            dst: slot_ref(raw.d)?,
            dst_path: path(program, &raw.p)?,
            base: operand(raw.a)?,
            path: path(program, &raw.s)?,
        },
        "store" => StmtKind::Store {
            target: operand(raw.a)?,
            path: if raw.s.is_empty() {
                vec![Step::Deref]
            } else {
                path(program, &raw.s)?
            },
            value: operand(raw.b)?,
        },
        "alloc" => StmtKind::Alloc {
            dst: slot_ref(raw.d)?,
        },
        "closure" => StmtKind::Closure {
            dst: slot_ref(raw.d)?,
            function: (!raw.f.is_empty()).then(|| Callee::External(raw.f.clone())),
            bindings: raw
                .args
                .iter()
                .map(|arg| operand(*arg))
                .collect::<Option<_>>()?,
        },
        "call" => StmtKind::Call {
            dst: slot_ref(raw.d),
            callees: raw
                .callees
                .iter()
                .map(|callee| Callee::External(callee.clone()))
                .collect(),
            algorithm: match raw.alg.as_str() {
                "static" => Algorithm::Static,
                "vta" => Algorithm::Vta,
                "cha" => Algorithm::Cha,
                _ => Algorithm::Unknown,
            },
            builtin: (!raw.builtin.is_empty()).then(|| raw.builtin.clone()),
            closure: slot_ref(raw.fv),
            args: raw
                .args
                .iter()
                .map(|arg| operand(*arg))
                .collect::<Option<_>>()?,
            mode: match raw.mode.as_str() {
                "go" => CallMode::Go,
                "defer" => CallMode::Defer,
                _ => CallMode::Call,
            },
            interface: (!raw.iface.is_empty()).then(|| raw.iface.clone()),
        },
        "ret" => StmtKind::Return {
            values: raw
                .args
                .iter()
                .map(|arg| operand(*arg))
                .collect::<Option<_>>()?,
        },
        _ => return None,
    };
    Some(Stmt { kind, pos })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(line: &str) -> GoFlowRowFrame {
        serde_json::from_str(line).expect("flow row parses")
    }

    const SINK_ROW: &str = r#"{"schema":"polint-go-semantic-4","kind":"flow_body","function":"example.test/app.entry","package_path":"example.test/app","file":"app.go","flow":{"params":[0],"slots":3,"names":[{"s":0,"n":"token"}],"ptypes":["string"],"classes":"..b","stmts":[{"o":"call","d":2,"args":[0],"callees":["example.test/app.sink"],"alg":"static","l":4,"c":2},{"o":"ret","l":5,"c":1}]}}"#;

    #[test]
    fn a_flow_row_lowers_into_a_body_with_its_calls() {
        let mut builder = GoFlowProgramBuilder::default();
        builder.add(&row(SINK_ROW), SINK_ROW);
        let program = builder.finish().expect("a program");
        let body = &program.program().functions[0];
        assert_eq!(body.name, "example.test/app.entry");
        assert_eq!(body.unit, "example.test/app");
        assert_eq!(body.params, vec![0]);
        assert_eq!(body.classes, b"..b".to_vec());
        let StmtKind::Call { dst, args, .. } = &body.stmts[0].kind else {
            panic!("the first statement is a call");
        };
        assert_eq!(*dst, Some(1));
        assert_eq!(args, &vec![Operand::Slot(0)]);
        assert_eq!(body.stmts[0].pos.line, 4);
        assert_eq!(program.digest().len(), 64);
    }

    #[test]
    fn a_body_whose_slots_fall_outside_it_is_dropped() {
        let broken = SINK_ROW.replace(r#""slots":3"#, r#""slots":1"#);
        let mut builder = GoFlowProgramBuilder::default();
        builder.add(&row(&broken), &broken);
        let program = builder.finish().expect("the row still counts");
        assert!(program.program().functions.is_empty());
    }

    #[test]
    fn no_flow_rows_is_no_program() {
        assert!(GoFlowProgramBuilder::default().finish().is_none());
    }

    #[test]
    fn the_digest_follows_the_rows() {
        let digest = |line: &str| {
            let mut builder = GoFlowProgramBuilder::default();
            builder.add(&row(line), line);
            builder.finish().expect("a program").digest().to_string()
        };
        let moved = SINK_ROW.replace(r#""l":4"#, r#""l":6"#);
        assert_ne!(digest(SINK_ROW), digest(&moved));
        assert_eq!(digest(SINK_ROW), digest(SINK_ROW));
    }
}

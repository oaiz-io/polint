//! Per-body indexes the solver reads: the moves between slots as edges, and
//! where each slot is passed, returned or bound, all indexed by slot.

use std::collections::{BTreeMap, HashMap};
use std::hash::{BuildHasherDefault, Hasher};

use crate::analysis_neutral::taint::ir::{
    Callee, FlowFunction, FlowProgram, FnId, NO_BODY, Operand, Slot, Step, StmtKind,
};
use crate::analysis_neutral::taint::path::Path;

/// A fast hasher for small integer-like keys (the hash `rustc` uses).
#[derive(Default, Clone, Copy)]
pub(crate) struct FxHasher {
    hash: u64,
}

const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

impl FxHasher {
    fn add(&mut self, value: u64) {
        self.hash = (self.hash.rotate_left(5) ^ value).wrapping_mul(SEED);
    }
}

impl Hasher for FxHasher {
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut value = 0u64;
            for (index, byte) in chunk.iter().enumerate() {
                value |= u64::from(*byte) << (8 * index);
            }
            self.add(value);
        }
    }

    fn write_u8(&mut self, value: u8) {
        self.add(u64::from(value));
    }

    fn write_u16(&mut self, value: u16) {
        self.add(u64::from(value));
    }

    fn write_u32(&mut self, value: u32) {
        self.add(u64::from(value));
    }

    fn write_u64(&mut self, value: u64) {
        self.add(value);
    }

    fn write_usize(&mut self, value: usize) {
        self.add(value as u64);
    }

    fn finish(&self) -> u64 {
        self.hash
    }
}

pub(crate) type FxBuild = BuildHasherDefault<FxHasher>;
pub(crate) type FxHashMap<K, V> = HashMap<K, V, FxBuild>;
pub(crate) type FxHashSet<K> = std::collections::HashSet<K, FxBuild>;

/// A move of values between two slot paths.
///
/// A value edge copies `src.src_path` (and everything below it) to
/// `dst.dst_path`; because both then refer to the same memory, a fact below a
/// memory step of the destination also holds at the source. An alias edge says
/// the two paths are the same place (`dst = &base.field`: `*dst` is
/// `base.field`), so facts cross it both ways.
#[derive(Clone, Debug)]
pub(crate) struct Edge {
    pub(crate) dst: Slot,
    pub(crate) dst_path: Path,
    pub(crate) src: Slot,
    pub(crate) src_path: Path,
    pub(crate) alias: bool,
    pub(crate) stmt: u32,
}

#[derive(Debug, Default)]
pub(crate) struct BodyIndex {
    pub(crate) edges: Vec<Edge>,
    /// Edges by source slot, and by destination slot.
    pub(crate) by_src: Vec<Vec<u32>>,
    pub(crate) by_dst: Vec<Vec<u32>>,
    /// Where a slot is a call argument: (call statement, argument position).
    pub(crate) arg_uses: Vec<Vec<(u32, u16)>>,
    /// Where a slot is the closure value a call calls.
    pub(crate) closure_uses: Vec<Vec<u32>>,
    /// Where a slot is returned: (statement, result position).
    pub(crate) returns: Vec<Vec<(u32, u16)>>,
    /// The parameter, free variable and global each slot is, if any.
    pub(crate) param_of: Vec<Option<u16>>,
    pub(crate) free_of: Vec<Option<u16>>,
    pub(crate) global_of: Vec<Option<u32>>,
    /// The closure statement that creates a closure value.
    pub(crate) closure_of: Vec<Option<u32>>,
    /// Field loads and field addresses, by field symbol: the slot holding the
    /// loaded value or the address.
    pub(crate) field_loads: BTreeMap<u32, Vec<Slot>>,
    pub(crate) field_addrs: BTreeMap<u32, Vec<Slot>>,
}

impl BodyIndex {
    pub(crate) fn new(function: &FlowFunction, global_ids: &BTreeMap<String, u32>) -> BodyIndex {
        let slots = function.slots as usize;
        let mut index = BodyIndex {
            by_src: vec![Vec::new(); slots],
            by_dst: vec![Vec::new(); slots],
            arg_uses: vec![Vec::new(); slots],
            closure_uses: vec![Vec::new(); slots],
            returns: vec![Vec::new(); slots],
            param_of: vec![None; slots],
            free_of: vec![None; slots],
            global_of: vec![None; slots],
            closure_of: vec![None; slots],
            ..BodyIndex::default()
        };
        for (position, slot) in function.params.iter().enumerate() {
            index.param_of[*slot as usize] = Some(position as u16);
        }
        for (position, slot) in function.free.iter().enumerate() {
            index.free_of[*slot as usize] = Some(position as u16);
        }
        for (slot, name) in &function.globals {
            index.global_of[*slot as usize] = global_ids.get(name).copied();
        }
        let path = |steps: &[Step]| Path::limited(steps.iter().copied(), usize::MAX);
        for (stmt_index, stmt) in function.stmts.iter().enumerate() {
            let stmt_id = stmt_index as u32;
            match &stmt.kind {
                StmtKind::Copy { dst, src } => {
                    if let Some(src) = src.slot() {
                        index.edge(*dst, Path::EMPTY, src, Path::EMPTY, false, stmt_id);
                    }
                }
                StmtKind::Load { dst, base, step } => {
                    if let Some(base) = base.slot() {
                        index.edge(*dst, Path::EMPTY, base, path(&[*step]), false, stmt_id);
                        if let Step::Field(sym) = step {
                            index.field_loads.entry(*sym).or_default().push(*dst);
                        }
                    }
                }
                StmtKind::Alias {
                    dst,
                    dst_path,
                    base,
                    path: base_path,
                } => {
                    if let Some(base) = base.slot() {
                        index.edge(*dst, path(dst_path), base, path(base_path), true, stmt_id);
                        if let (Some(Step::Field(sym)), [Step::Deref]) =
                            (base_path.last(), dst_path.as_slice())
                        {
                            index.field_addrs.entry(*sym).or_default().push(*dst);
                        }
                    }
                }
                StmtKind::Store {
                    target,
                    path: target_path,
                    value,
                } => {
                    if let (Some(target), Some(value)) = (target.slot(), value.slot()) {
                        index.edge(
                            target,
                            path(target_path),
                            value,
                            Path::EMPTY,
                            false,
                            stmt_id,
                        );
                    }
                }
                StmtKind::Alloc { .. } => {}
                StmtKind::Closure {
                    dst,
                    function: closure,
                    bindings,
                } => {
                    index.closure_of[*dst as usize] = Some(stmt_id);
                    let body = match closure {
                        Some(Callee::Body(id)) => *id,
                        _ => NO_BODY,
                    };
                    for (position, binding) in bindings.iter().enumerate() {
                        if let Some(binding) = binding.slot() {
                            index.edge(
                                *dst,
                                path(&[Step::Capture(body, position as u16)]),
                                binding,
                                Path::EMPTY,
                                false,
                                stmt_id,
                            );
                        }
                    }
                }
                StmtKind::Call { args, closure, .. } => {
                    for (position, arg) in args.iter().enumerate() {
                        if let Operand::Slot(slot) = arg {
                            index.arg_uses[*slot as usize].push((stmt_id, position as u16));
                        }
                    }
                    if let Some(closure) = closure {
                        index.closure_uses[*closure as usize].push(stmt_id);
                    }
                }
                StmtKind::Return { values } => {
                    for (position, value) in values.iter().enumerate() {
                        if let Operand::Slot(slot) = value {
                            index.returns[*slot as usize].push((stmt_id, position as u16));
                        }
                    }
                }
            }
        }
        index
    }

    fn edge(
        &mut self,
        dst: Slot,
        dst_path: Path,
        src: Slot,
        src_path: Path,
        alias: bool,
        stmt: u32,
    ) {
        let id = self.edges.len() as u32;
        self.edges.push(Edge {
            dst,
            dst_path,
            src,
            src_path,
            alias,
            stmt,
        });
        self.by_src[src as usize].push(id);
        self.by_dst[dst as usize].push(id);
    }
}

/// Indexes over the whole program: each body's index, the calls of each body
/// (for returning taint to callers), globals and units.
#[derive(Debug, Default)]
pub(crate) struct ProgramIndex {
    pub(crate) bodies: Vec<BodyIndex>,
    /// For each body, the call statements that may call it: (caller, statement).
    pub(crate) callers: Vec<Vec<(FnId, u32)>>,
    /// Globals by id, and for each the bodies that address it, with the slot.
    pub(crate) globals: Vec<String>,
    pub(crate) global_users: Vec<Vec<(FnId, Slot)>>,
    /// Units (packages) by id, and each body's unit.
    pub(crate) units: Vec<String>,
    pub(crate) unit_of: Vec<u32>,
    /// For each body, the bodies that create a closure of it.
    pub(crate) creators: Vec<Vec<FnId>>,
}

impl ProgramIndex {
    pub(crate) fn new(program: &FlowProgram) -> ProgramIndex {
        let mut global_ids = BTreeMap::new();
        for function in &program.functions {
            for name in function.globals.values() {
                let next = global_ids.len() as u32;
                global_ids.entry(name.clone()).or_insert(next);
            }
        }
        let mut globals = vec![String::new(); global_ids.len()];
        for (name, id) in &global_ids {
            globals[*id as usize] = name.clone();
        }
        let mut unit_ids = BTreeMap::new();
        for function in &program.functions {
            let next = unit_ids.len() as u32;
            unit_ids.entry(function.unit.clone()).or_insert(next);
        }
        let mut units = vec![String::new(); unit_ids.len()];
        for (name, id) in &unit_ids {
            units[*id as usize] = name.clone();
        }
        let mut index = ProgramIndex {
            bodies: program
                .functions
                .iter()
                .map(|function| BodyIndex::new(function, &global_ids))
                .collect(),
            callers: vec![Vec::new(); program.functions.len()],
            creators: vec![Vec::new(); program.functions.len()],
            global_users: vec![Vec::new(); globals.len()],
            globals,
            unit_of: program
                .functions
                .iter()
                .map(|function| unit_ids[&function.unit])
                .collect(),
            units,
        };
        for (caller, function) in program.functions.iter().enumerate() {
            for (stmt_index, stmt) in function.stmts.iter().enumerate() {
                match &stmt.kind {
                    StmtKind::Call { callees, .. } => {
                        for callee in callees {
                            if let Callee::Body(callee) = callee {
                                index.callers[*callee as usize]
                                    .push((caller as FnId, stmt_index as u32));
                            }
                        }
                    }
                    StmtKind::Closure {
                        function: Some(Callee::Body(closure)),
                        ..
                    } => index.creators[*closure as usize].push(caller as FnId),
                    _ => {}
                }
            }
            for (slot, name) in &function.globals {
                index.global_users[global_ids[name] as usize].push((caller as FnId, *slot));
            }
        }
        index
    }
}

//! The flow program the data-flow solver reads: per function body, numbered
//! value slots and the statements that move values between them. A frontend
//! lowers its own IR into this shape (Go: the semantic sidecar's SSA).

use std::collections::BTreeMap;

/// A function body's index in its [`FlowProgram`].
pub(crate) type FnId = u32;
/// A value slot of one function body.
pub(crate) type Slot = u32;
/// An interned struct field name.
pub(crate) type Sym = u32;

/// One step of an access path below a slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Step {
    /// A struct field.
    Field(Sym),
    /// An element of a slice, array, map, string or channel.
    Elem,
    /// What a pointer points to.
    Deref,
    /// The Nth value of a tuple (a call with several results).
    Result(u16),
    /// The Nth free-variable binding of a closure of the body with this id
    /// ([`NO_BODY`] when the program has no body for it).
    Capture(FnId, u16),
}

/// The function id a capture step names when the closure's body is unknown.
pub(crate) const NO_BODY: FnId = FnId::MAX;

impl Step {
    /// A step into memory another value may share: through a pointer, or into
    /// the elements a slice, map or channel refers to.
    pub(crate) fn is_memory(self) -> bool {
        matches!(self, Step::Deref | Step::Elem)
    }
}

/// An operand: a slot, or a constant of the body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Operand {
    Slot(Slot),
    Const(u32),
}

impl Operand {
    pub(crate) fn slot(self) -> Option<Slot> {
        match self {
            Operand::Slot(slot) => Some(slot),
            Operand::Const(_) => None,
        }
    }
}

/// How a call's candidate callees were found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Algorithm {
    /// The one callee the call names.
    Static,
    /// Variable-type analysis.
    Vta,
    /// Class-hierarchy analysis.
    Cha,
    /// No candidate is known.
    Unknown,
}

/// A candidate callee: a body in the program, or a function the program does
/// not have the body of, named.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Callee {
    Body(FnId),
    External(String),
}

/// How the call is made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub(crate) enum CallMode {
    #[default]
    Call,
    Go,
    Defer,
}

/// Where a statement is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub(crate) struct Pos {
    pub(crate) line: u32,
    pub(crate) col: u32,
    /// The call expression's byte span, for a call.
    pub(crate) start_byte: u32,
    pub(crate) end_byte: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StmtKind {
    /// `dst` gets everything `src` has.
    Copy { dst: Slot, src: Operand },
    /// `dst` gets what is at `base.step`.
    Load {
        dst: Slot,
        base: Operand,
        step: Step,
    },
    /// `dst.dst_path` is the same place as `base.path`: an address
    /// (`dst = &base.field`: `dst_path` is `[Deref]`, `path` `[Deref, Field]`),
    /// or a slice of an array (`dst_path` `[Elem]`, `path` `[Deref, Elem]`).
    Alias {
        dst: Slot,
        dst_path: Vec<Step>,
        base: Operand,
        path: Vec<Step>,
    },
    /// `target.path` gets `value` (`path` is `[Deref]` for `*target = value`).
    Store {
        target: Operand,
        path: Vec<Step>,
        value: Operand,
    },
    /// `dst` is a new memory location.
    Alloc { dst: Slot },
    /// `dst` is a closure of `function` binding `bindings` to its free variables.
    Closure {
        dst: Slot,
        function: Option<Callee>,
        bindings: Vec<Operand>,
    },
    /// A call: `dst` is its result (a tuple when the callee has several), `args`
    /// its arguments (a method's receiver first), `closure` the called closure
    /// value whose bindings are the callee's free variables.
    Call {
        dst: Option<Slot>,
        callees: Vec<Callee>,
        algorithm: Algorithm,
        builtin: Option<String>,
        closure: Option<Slot>,
        args: Vec<Operand>,
        mode: CallMode,
        /// For an interface call, the interface method (`(pkg.Iface).Method`).
        interface: Option<String>,
    },
    /// The body returns `values`, one per result.
    Return { values: Vec<Operand> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Stmt {
    pub(crate) kind: StmtKind,
    pub(crate) pos: Pos,
}

/// One function body.
#[derive(Clone, Debug, Default)]
pub(crate) struct FlowFunction {
    /// The qualified name (`example.com/app.F`, `(*example.com/app.T).M`).
    pub(crate) name: String,
    /// The package (the unit) the body belongs to.
    pub(crate) unit: String,
    /// The file the body is written in, as the frontend names it.
    pub(crate) file: String,
    pub(crate) params: Vec<Slot>,
    pub(crate) param_types: Vec<String>,
    pub(crate) free: Vec<Slot>,
    pub(crate) results: u16,
    pub(crate) slots: u32,
    /// Names of named slots: parameters, free variables, address-taken locals.
    pub(crate) names: BTreeMap<Slot, String>,
    /// The global each global-address slot stands for.
    pub(crate) globals: BTreeMap<Slot, String>,
    pub(crate) consts: Vec<String>,
    /// The kind of value each slot's type holds (`c` a context, `b` a boolean,
    /// `n` a number, `.` anything else); empty when every slot is `.`.
    pub(crate) classes: Vec<u8>,
    pub(crate) stmts: Vec<Stmt>,
}

/// Kinds of values a query can declare unable to carry what it tracks: a value
/// of such a kind never holds taint, and never reaches a sink.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ValueKinds(u8);

impl ValueKinds {
    pub(crate) const NONE: ValueKinds = ValueKinds(0);
    /// `context.Context` values.
    pub(crate) const CONTEXT: ValueKinds = ValueKinds(1);
    /// Booleans.
    pub(crate) const BOOLEAN: ValueKinds = ValueKinds(2);
    /// Numbers (integers, floats, complex numbers, and named types of them).
    pub(crate) const NUMBER: ValueKinds = ValueKinds(4);

    pub(crate) fn union(self, other: ValueKinds) -> ValueKinds {
        ValueKinds(self.0 | other.0)
    }

    /// Whether a slot of this class is one of these kinds.
    pub(crate) fn contains_class(self, class: u8) -> bool {
        let kind = match class {
            b'c' => ValueKinds::CONTEXT,
            b'b' => ValueKinds::BOOLEAN,
            b'n' => ValueKinds::NUMBER,
            _ => return false,
        };
        self.0 & kind.0 != 0
    }
}

impl FlowFunction {
    /// Whether the body is a method: its first parameter is the receiver.
    pub(crate) fn is_method(&self) -> bool {
        self.name.starts_with('(')
    }
}

/// Every function body of a program, with the field names their paths use.
#[derive(Clone, Debug, Default)]
pub(crate) struct FlowProgram {
    pub(crate) functions: Vec<FlowFunction>,
    pub(crate) by_name: BTreeMap<String, FnId>,
    pub(crate) symbols: Vec<String>,
    symbol_ids: BTreeMap<String, Sym>,
}

impl FlowProgram {
    pub(crate) fn symbol(&mut self, name: &str) -> Sym {
        if let Some(id) = self.symbol_ids.get(name) {
            return *id;
        }
        let id = self.symbols.len() as Sym;
        self.symbols.push(name.to_string());
        self.symbol_ids.insert(name.to_string(), id);
        id
    }

    pub(crate) fn symbol_name(&self, sym: Sym) -> &str {
        self.symbols
            .get(sym as usize)
            .map(String::as_str)
            .unwrap_or("")
    }

    /// Adds a body; a second body with the same name keeps the first.
    pub(crate) fn add(&mut self, function: FlowFunction) -> FnId {
        if let Some(id) = self.by_name.get(&function.name) {
            return *id;
        }
        let id = self.functions.len() as FnId;
        self.by_name.insert(function.name.clone(), id);
        self.functions.push(function);
        id
    }

    pub(crate) fn function(&self, id: FnId) -> &FlowFunction {
        &self.functions[id as usize]
    }

    /// Callees named before their bodies were added become body references.
    pub(crate) fn link(&mut self) {
        let by_name = self.by_name.clone();
        let link = |callee: &mut Callee| {
            if let Callee::External(name) = callee
                && let Some(id) = by_name.get(name.as_str())
            {
                *callee = Callee::Body(*id);
            }
        };
        for function in &mut self.functions {
            for stmt in &mut function.stmts {
                match &mut stmt.kind {
                    StmtKind::Call { callees, .. } => callees.iter_mut().for_each(link),
                    StmtKind::Closure {
                        function: Some(callee),
                        ..
                    } => link(callee),
                    _ => {}
                }
            }
        }
    }
}

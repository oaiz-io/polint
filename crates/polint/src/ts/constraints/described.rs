//! Which values the lowered program describes completely.
//!
//! Difference propagation follows copies, fields and calls across files. An
//! unknown value opens the places it reaches; a value passed to unknown code
//! escapes. Prototype lookup runs after ordinary propagation, respects own
//! fields, and records the inherited slots the graph must also read.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use super::{FlowObject, FlowRow, FlowSpan, FlowVar, TsValueFlows};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Obj {
    Site(FlowSpan),
    Prototype(FlowSpan),
    Namespace,
    Module,
    Exports,
}

impl From<FlowObject> for Obj {
    fn from(object: FlowObject) -> Self {
        match object {
            FlowObject::Site(span) => Self::Site(span),
            FlowObject::Namespace => Self::Namespace,
            FlowObject::Module => Self::Module,
            FlowObject::Exports => Self::Exports,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Token {
    Unknown,
    Prim,
    Function(u32, FlowSpan),
    Object(u32, Obj),
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Described {
    pub(crate) named: BTreeMap<FlowSpan, u8>,
    pub(crate) open: BTreeSet<FlowVar>,
    pub(crate) escaped_functions: BTreeSet<FlowSpan>,
    pub(crate) escaped_objects: BTreeSet<Obj>,
    pub(crate) lookups: BTreeSet<(usize, Token)>,
    pub(crate) prototypes: BTreeSet<FlowSpan>,
}

/// Solves every file together. A request is linked only to a lowered file;
/// imports whose target cannot be lowered remain unknown.
pub(crate) fn describe_program(
    files: &[&TsValueFlows],
    has_node: impl Fn(usize, FlowSpan) -> bool,
    request_target: impl Fn(usize, u32) -> Option<usize>,
) -> Vec<Described> {
    let mut solve = Solve::new(files);
    solve.seed(has_node, request_target);
    solve.run();
    solve.finish()
}

#[cfg(test)]
pub(crate) fn describe(flows: &TsValueFlows, has_node: impl Fn(FlowSpan) -> bool) -> Described {
    describe_program(&[flows], |_, span| has_node(span), |_, _| None)
        .pop()
        .unwrap_or_default()
}

#[derive(Clone, Copy)]
struct Load {
    file: usize,
    row: usize,
    dst: usize,
    field: usize,
}

#[derive(Clone, Copy)]
struct Store {
    src: usize,
    field: usize,
}

#[derive(Clone)]
struct Call {
    result: usize,
    receiver: Option<usize>,
    arguments: Vec<usize>,
    spread_from: Option<usize>,
}

struct Solve<'a> {
    files: &'a [&'a TsValueFlows],
    functions: HashMap<(u32, FlowSpan), u8>,
    index: HashMap<(usize, FlowVar), usize>,
    vars: Vec<Option<(usize, FlowVar)>>,
    sets: Vec<BTreeSet<Token>>,
    delta: Vec<Vec<Token>>,
    queue: VecDeque<usize>,
    successors: Vec<Vec<usize>>,
    edges: HashSet<(usize, usize)>,
    loads_by_base: Vec<Vec<Load>>,
    stores_by_base: Vec<Vec<Store>>,
    calls_by_callee: Vec<Vec<Call>>,
    fields: HashMap<Box<str>, usize>,
    slots: HashMap<(Token, usize), usize>,
    slots_of: HashMap<Token, Vec<usize>>,
    stored: HashSet<(Token, usize)>,
    loads_of: HashMap<Token, Vec<usize>>,
    lookups: Vec<(Load, Token)>,
    escaped: HashSet<Token>,
    exported: HashSet<Token>,
    descriptions: Vec<Described>,
    sink: usize,
    export: usize,
    prototype: usize,
    proto: usize,
}

impl<'a> Solve<'a> {
    fn new(files: &'a [&'a TsValueFlows]) -> Self {
        let mut solve = Self {
            files,
            functions: HashMap::new(),
            index: HashMap::new(),
            vars: Vec::new(),
            sets: Vec::new(),
            delta: Vec::new(),
            queue: VecDeque::new(),
            successors: Vec::new(),
            edges: HashSet::new(),
            loads_by_base: Vec::new(),
            stores_by_base: Vec::new(),
            calls_by_callee: Vec::new(),
            fields: HashMap::new(),
            slots: HashMap::new(),
            slots_of: HashMap::new(),
            stored: HashSet::new(),
            loads_of: HashMap::new(),
            lookups: Vec::new(),
            escaped: HashSet::new(),
            exported: HashSet::new(),
            descriptions: files.iter().map(|_| Described::default()).collect(),
            sink: 0,
            export: 0,
            prototype: 0,
            proto: 0,
        };
        solve.sink = solve.var(0, FlowVar::Sink);
        solve.export = solve.node(None);
        solve.prototype = solve.field("exact:prototype");
        solve.proto = solve.field("$proto");
        solve
    }

    fn node(&mut self, var: Option<(usize, FlowVar)>) -> usize {
        let node = self.sets.len();
        self.vars.push(var);
        self.sets.push(BTreeSet::new());
        self.delta.push(Vec::new());
        self.successors.push(Vec::new());
        self.loads_by_base.push(Vec::new());
        self.stores_by_base.push(Vec::new());
        self.calls_by_callee.push(Vec::new());
        node
    }

    fn var(&mut self, file: usize, var: FlowVar) -> usize {
        let file = if matches!(var, FlowVar::Open | FlowVar::Sink | FlowVar::Prim) {
            usize::MAX
        } else {
            file
        };
        if let Some(&node) = self.index.get(&(file, var)) {
            return node;
        }
        let node = self.node(Some((file, var)));
        self.index.insert((file, var), node);
        node
    }

    fn field(&mut self, field: &str) -> usize {
        let next = self.fields.len();
        *self.fields.entry(field.into()).or_insert(next)
    }

    fn add(&mut self, node: usize, token: Token) {
        if self.sets[node].insert(token) {
            if self.delta[node].is_empty() {
                self.queue.push_back(node);
            }
            self.delta[node].push(token);
        }
    }

    fn edge(&mut self, src: usize, dst: usize) {
        if src != dst && self.edges.insert((src, dst)) {
            self.successors[src].push(dst);
            for token in self.sets[src].clone() {
                self.add(dst, token);
            }
        }
    }

    fn slot(&mut self, token: Token, field: usize) -> usize {
        if let Some(&node) = self.slots.get(&(token, field)) {
            return node;
        }
        let node = self.node(None);
        self.slots.insert((token, field), node);
        self.slots_of.entry(token).or_default().push(node);
        if self.escaped.contains(&token) {
            self.edge(node, self.sink);
        }
        if self.exported.contains(&token)
            || matches!(
                token,
                Token::Object(_, Obj::Namespace | Obj::Module | Obj::Exports)
            )
        {
            self.edge(node, self.export);
        }
        if field == self.prototype
            && let Token::Function(file, span) = token
        {
            self.add(node, Token::Object(file, Obj::Prototype(span)));
            self.stored.insert((token, field));
            self.descriptions[file as usize].prototypes.insert(span);
        }
        node
    }

    fn seed(
        &mut self,
        has_node: impl Fn(usize, FlowSpan) -> bool,
        request_target: impl Fn(usize, u32) -> Option<usize>,
    ) {
        let open = self.var(0, FlowVar::Open);
        self.add(open, Token::Unknown);
        let prim = self.var(0, FlowVar::Prim);
        self.add(prim, Token::Prim);
        for (file, flows) in self.files.iter().enumerate() {
            if !flows.lowered {
                continue;
            }
            for function in &flows.functions {
                if has_node(file, function.span) {
                    self.functions
                        .insert((file as u32, function.span), function.params);
                }
            }
        }
        for (file, flows) in self.files.iter().enumerate() {
            if !flows.lowered {
                continue;
            }
            let mut written = BTreeSet::new();
            let mut read = BTreeSet::new();
            for (row, flow) in flows.rows.iter().enumerate() {
                match flow {
                    FlowRow::Copy { dst, src } => {
                        written.insert(*dst);
                        read.insert(*src);
                        let (dst, src) = (self.var(file, *dst), self.var(file, *src));
                        self.edge(src, dst);
                    }
                    FlowRow::Alloc { dst, object } => {
                        written.insert(*dst);
                        let dst = self.var(file, *dst);
                        self.add(dst, Token::Object(file as u32, (*object).into()));
                    }
                    FlowRow::Load { dst, base, field } => {
                        written.insert(*dst);
                        read.insert(*base);
                        let (dst, base, field) = (
                            self.var(file, *dst),
                            self.var(file, *base),
                            self.field(field),
                        );
                        self.loads_by_base[base].push(Load {
                            file,
                            row,
                            dst,
                            field,
                        });
                    }
                    FlowRow::Store { base, field, src } => {
                        read.insert(*base);
                        read.insert(*src);
                        let (base, src, field) = (
                            self.var(file, *base),
                            self.var(file, *src),
                            self.field(field),
                        );
                        self.stores_by_base[base].push(Store { src, field });
                    }
                }
            }
            for call in &flows.calls {
                written.insert(call.result);
                read.insert(call.callee);
                read.extend(call.receiver);
                read.extend(call.arguments.iter().copied());
                let callee = self.var(file, call.callee);
                let call = Call {
                    result: self.var(file, call.result),
                    receiver: call.receiver.map(|var| self.var(file, var)),
                    arguments: call
                        .arguments
                        .iter()
                        .map(|&var| self.var(file, var))
                        .collect(),
                    spread_from: call.spread_from,
                };
                self.calls_by_callee[callee].push(call);
            }
            for &var in read.union(&written) {
                let node = self.var(file, var);
                match var {
                    FlowVar::Function(span) => self.add(
                        node,
                        if self.functions.contains_key(&(file as u32, span)) {
                            Token::Function(file as u32, span)
                        } else {
                            Token::Unknown
                        },
                    ),
                    FlowVar::RequestNamespace(request) | FlowVar::RequestModule(request) => {
                        if let Some(target) = request_target(file, request)
                            .filter(|&target| self.files.get(target).is_some_and(|f| f.lowered))
                        {
                            let target_var = if matches!(var, FlowVar::RequestNamespace(_)) {
                                FlowVar::Namespace
                            } else {
                                FlowVar::Module
                            };
                            let src = self.var(target, target_var);
                            self.edge(src, node);
                        } else {
                            self.add(node, Token::Unknown);
                        }
                    }
                    FlowVar::Local(_) | FlowVar::Temp(_) if !written.contains(&var) => {
                        self.add(node, Token::Unknown)
                    }
                    _ => {}
                }
            }
            for function in &flows.functions {
                if !self.functions.contains_key(&(file as u32, function.span)) {
                    self.open_signature(file as u32, function.span, function.params, self.sink);
                }
            }
        }
    }

    fn open_signature(&mut self, file: u32, span: FlowSpan, params: u8, boundary: usize) {
        for position in 0..params {
            let param = self.var(file as usize, FlowVar::Param(span, position));
            self.add(param, Token::Unknown);
        }
        let this = self.var(file as usize, FlowVar::This(span));
        self.add(this, Token::Unknown);
        let ret = self.var(file as usize, FlowVar::Return(span));
        self.edge(ret, boundary);
    }

    fn boundary(&mut self, token: Token, escape: bool) {
        let fresh = if escape {
            self.escaped.insert(token)
        } else {
            self.exported.insert(token)
        };
        if !fresh {
            return;
        }
        let boundary = if escape { self.sink } else { self.export };
        if let Token::Function(file, span) = token {
            let params = self.functions.get(&(file, span)).copied().unwrap_or(0);
            self.open_signature(file, span, params, boundary);
            if escape {
                self.descriptions[file as usize]
                    .escaped_functions
                    .insert(span);
            }
        } else if let Token::Object(file, object) = token
            && escape
        {
            self.descriptions[file as usize]
                .escaped_objects
                .insert(object);
        }
        // The module boundary allows outside calls but assumes outside code
        // neither rewrites exports nor writes fields of exported objects.
        for slot in self.slots_of.get(&token).cloned().unwrap_or_default() {
            self.edge(slot, boundary);
        }
        if escape {
            for dst in self.loads_of.get(&token).cloned().unwrap_or_default() {
                self.add(dst, Token::Unknown);
            }
        }
    }

    fn load(&mut self, load: Load, token: Token) {
        if matches!(token, Token::Unknown | Token::Prim) {
            self.add(load.dst, Token::Unknown);
            return;
        }
        let slot = self.slot(token, load.field);
        self.edge(slot, load.dst);
        self.loads_of.entry(token).or_default().push(load.dst);
        self.lookups.push((load, token));
        if self.escaped.contains(&token) {
            self.add(load.dst, Token::Unknown);
        }
    }

    fn store(&mut self, store: Store, token: Token) {
        if matches!(token, Token::Unknown | Token::Prim) {
            self.edge(store.src, self.sink);
        } else {
            let slot = self.slot(token, store.field);
            self.stored.insert((token, store.field));
            self.edge(store.src, slot);
        }
    }

    fn call(&mut self, call: &Call, token: Token) {
        let Token::Function(file, span) = token else {
            if token != Token::Prim {
                for &src in call.receiver.iter().chain(&call.arguments) {
                    self.edge(src, self.sink);
                }
                self.add(call.result, Token::Unknown);
            }
            return;
        };
        let params = self.functions.get(&(file, span)).copied().unwrap_or(0);
        let this = self.var(file as usize, FlowVar::This(span));
        match call.receiver {
            Some(receiver) => self.edge(receiver, this),
            None => self.add(this, Token::Unknown),
        }
        let bound = call.spread_from.unwrap_or(call.arguments.len());
        for (position, &arg) in call.arguments.iter().enumerate() {
            if position < bound && position < usize::from(params) {
                let param = self.var(file as usize, FlowVar::Param(span, position as u8));
                self.edge(arg, param);
            } else {
                self.edge(arg, self.sink);
            }
        }
        if call.spread_from.is_some() {
            for position in bound.min(usize::from(params))..usize::from(params) {
                let param = self.var(file as usize, FlowVar::Param(span, position as u8));
                self.add(param, Token::Unknown);
            }
        }
        let ret = self.var(file as usize, FlowVar::Return(span));
        self.edge(ret, call.result);
    }

    fn propagate(&mut self) {
        while let Some(node) = self.queue.pop_front() {
            let delta = std::mem::take(&mut self.delta[node]);
            for token in delta {
                for dst in self.successors[node].clone() {
                    self.add(dst, token);
                }
                for load in self.loads_by_base[node].clone() {
                    self.load(load, token);
                }
                for store in self.stores_by_base[node].clone() {
                    self.store(store, token);
                }
                for call in self.calls_by_callee[node].clone() {
                    self.call(&call, token);
                }
                if node == self.sink {
                    self.boundary(token, true);
                }
                if node == self.export {
                    self.boundary(token, false);
                }
            }
        }
    }

    fn lookup(&mut self, load: Load, token: Token, depth: u8, visited: &mut HashSet<Token>) {
        if depth >= 16 || !visited.insert(token) || matches!(token, Token::Unknown | Token::Prim) {
            self.add(load.dst, Token::Unknown);
            return;
        }
        // Outside code may shadow any field of an escaped object, so the
        // inherited value is a candidate beside an unknown one.
        if self.escaped.contains(&token) {
            self.add(load.dst, Token::Unknown);
        }
        if self.stored.contains(&(token, load.field)) {
            let slot = self.slot(token, load.field);
            self.edge(slot, load.dst);
            self.descriptions[load.file]
                .lookups
                .insert((load.row, token));
        } else if let Some(&proto) = self.slots.get(&(token, self.proto))
            && !self.sets[proto].is_empty()
        {
            for parent in self.sets[proto].clone() {
                self.lookup(load, parent, depth + 1, visited);
            }
        } else if !matches!(token, Token::Object(_, Obj::Namespace)) {
            self.add(load.dst, Token::Unknown);
            if matches!(token, Token::Function(..)) {
                self.add(self.sink, token);
            }
        }
        visited.remove(&token);
    }

    fn run(&mut self) {
        loop {
            self.propagate();
            for (load, token) in self.lookups.clone() {
                if !self.stored.contains(&(token, load.field)) {
                    self.lookup(load, token, 0, &mut HashSet::new());
                }
            }
            if self.queue.is_empty() {
                break;
            }
        }
    }

    fn finish(mut self) -> Vec<Described> {
        for (file, flows) in self.files.iter().enumerate() {
            if !flows.lowered {
                continue;
            }
            let mut sites = BTreeMap::<FlowSpan, Option<u8>>::new();
            for call in &flows.calls {
                let callee_file =
                    if matches!(call.callee, FlowVar::Open | FlowVar::Sink | FlowVar::Prim) {
                        usize::MAX
                    } else {
                        file
                    };
                let set = &self.sets[self.index[&(callee_file, call.callee)]];
                let mut pad = None::<u8>;
                for token in set {
                    match token {
                        Token::Function(file, span) => {
                            let params = self.functions.get(&(*file, *span)).copied().unwrap_or(0);
                            pad = Some(pad.map_or(params, |held| held.max(params)));
                        }
                        Token::Prim => {}
                        _ => {
                            pad = None;
                            break;
                        }
                    }
                }
                let entry = sites.entry(call.site).or_insert(Some(0));
                *entry = entry.zip(pad).map(|(held, pad)| held.max(pad));
            }
            self.descriptions[file].named = sites
                .into_iter()
                .filter_map(|(site, pad)| Some((site, pad?)))
                .collect();
        }
        for (var, set) in self.vars.iter().zip(&self.sets) {
            if let Some((file, var)) = var
                && *file != usize::MAX
                && set.contains(&Token::Unknown)
            {
                self.descriptions[*file].open.insert(*var);
            }
        }
        std::mem::take(&mut self.descriptions)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ts::constraints::{FlowCall, LoweredFunction};

    const F: FlowSpan = (10, 20);
    const G: FlowSpan = (30, 40);
    const SITE: FlowSpan = (50, 55);
    const OBJ: FlowSpan = (60, 70);

    fn flows(rows: Vec<FlowRow>, calls: Vec<FlowCall>) -> TsValueFlows {
        TsValueFlows {
            lowered: true,
            rows,
            calls,
            functions: vec![
                LoweredFunction { span: F, params: 1 },
                LoweredFunction { span: G, params: 0 },
            ],
            ..TsValueFlows::default()
        }
    }

    fn call(callee: FlowVar, arguments: Vec<FlowVar>) -> FlowCall {
        FlowCall {
            site: SITE,
            callee,
            receiver: None,
            arguments,
            spread_from: None,
            result: FlowVar::Temp(99),
            constructs: None,
        }
    }

    fn copy(dst: FlowVar, src: FlowVar) -> FlowRow {
        FlowRow::Copy { dst, src }
    }

    #[test]
    fn a_local_function_callee_is_named_with_its_parameter_count() {
        let described = describe(
            &flows(
                vec![copy(FlowVar::Local(0), FlowVar::Function(F))],
                vec![call(FlowVar::Local(0), Vec::new())],
            ),
            |_| true,
        );
        assert_eq!(described.named, BTreeMap::from([(SITE, 1)]));
        assert!(described.escaped_functions.is_empty());
    }

    #[test]
    fn an_unwritten_local_is_unknown_and_its_site_is_not_named() {
        let described = describe(
            &flows(
                vec![copy(FlowVar::Local(0), FlowVar::Function(F))],
                vec![call(FlowVar::Local(1), vec![FlowVar::Local(0)])],
            ),
            |_| true,
        );
        assert!(described.named.is_empty());
        assert_eq!(described.escaped_functions, BTreeSet::from([F]));
        assert!(described.open.contains(&FlowVar::Param(F, 0)));
    }

    #[test]
    fn a_function_without_a_node_is_unknown() {
        let described = describe(
            &flows(
                vec![copy(FlowVar::Local(0), FlowVar::Function(F))],
                vec![call(FlowVar::Local(0), Vec::new())],
            ),
            |span| span != F,
        );
        assert!(described.named.is_empty());
    }

    #[test]
    fn a_callback_passed_to_a_parameter_is_named_through_the_parameter() {
        // f(g) where f calls its parameter: the inner site is described.
        let inner = FlowCall {
            site: (12, 14),
            ..call(FlowVar::Local(2), Vec::new())
        };
        let described = describe(
            &flows(
                vec![
                    copy(FlowVar::Local(0), FlowVar::Function(F)),
                    copy(FlowVar::Local(2), FlowVar::Param(F, 0)),
                ],
                vec![call(FlowVar::Local(0), vec![FlowVar::Function(G)]), inner],
            ),
            |_| true,
        );
        assert_eq!(described.named.get(&(12, 14)), Some(&0));
        assert!(!described.open.contains(&FlowVar::Param(F, 0)));
    }

    #[test]
    fn a_stored_method_is_named_and_an_unstored_field_is_unknown() {
        let rows = vec![
            FlowRow::Alloc {
                dst: FlowVar::Local(0),
                object: FlowObject::Site(OBJ),
            },
            FlowRow::Store {
                base: FlowVar::Local(0),
                field: "exact:run".into(),
                src: FlowVar::Function(G),
            },
            FlowRow::Load {
                dst: FlowVar::Temp(1),
                base: FlowVar::Local(0),
                field: "exact:run".into(),
            },
            FlowRow::Load {
                dst: FlowVar::Temp(2),
                base: FlowVar::Local(0),
                field: "exact:toString".into(),
            },
        ];
        let described = describe(
            &flows(rows, vec![call(FlowVar::Temp(1), Vec::new())]),
            |_| true,
        );
        assert_eq!(described.named, BTreeMap::from([(SITE, 0)]));
        assert!(described.open.contains(&FlowVar::Temp(2)));
        assert!(!described.open.contains(&FlowVar::Temp(1)));
    }

    #[test]
    fn an_escaped_object_exposes_its_slots() {
        let rows = vec![
            FlowRow::Alloc {
                dst: FlowVar::Local(0),
                object: FlowObject::Site(OBJ),
            },
            FlowRow::Store {
                base: FlowVar::Local(0),
                field: "exact:run".into(),
                src: FlowVar::Function(F),
            },
            copy(FlowVar::Sink, FlowVar::Local(0)),
        ];
        let described = describe(&flows(rows, Vec::new()), |_| true);
        assert_eq!(described.escaped_objects, BTreeSet::from([Obj::Site(OBJ)]));
        assert_eq!(described.escaped_functions, BTreeSet::from([F]));
        assert!(described.open.contains(&FlowVar::Param(F, 0)));
    }

    #[test]
    fn an_escaped_object_inherits_its_prototype_methods_beside_an_unknown_value() {
        const PROTO: FlowSpan = (80, 90);
        let rows = vec![
            FlowRow::Alloc {
                dst: FlowVar::Local(1),
                object: FlowObject::Site(PROTO),
            },
            FlowRow::Store {
                base: FlowVar::Local(1),
                field: "exact:run".into(),
                src: FlowVar::Function(G),
            },
            FlowRow::Alloc {
                dst: FlowVar::Local(0),
                object: FlowObject::Site(OBJ),
            },
            FlowRow::Store {
                base: FlowVar::Local(0),
                field: "$proto".into(),
                src: FlowVar::Local(1),
            },
            copy(FlowVar::Sink, FlowVar::Local(0)),
            FlowRow::Load {
                dst: FlowVar::Temp(1),
                base: FlowVar::Local(0),
                field: "exact:run".into(),
            },
        ];
        let described = describe(
            &flows(rows, vec![call(FlowVar::Temp(1), Vec::new())]),
            |_| true,
        );
        assert!(described.named.is_empty());
        assert!(described.open.contains(&FlowVar::Temp(1)));
        assert!(
            described
                .lookups
                .contains(&(5, Token::Object(0, Obj::Site(PROTO))))
        );
    }

    #[test]
    fn reading_a_property_of_a_function_escapes_it() {
        let rows = vec![
            copy(FlowVar::Local(0), FlowVar::Function(F)),
            FlowRow::Load {
                dst: FlowVar::Temp(1),
                base: FlowVar::Local(0),
                field: "exact:call".into(),
            },
        ];
        let described = describe(&flows(rows, Vec::new()), |_| true);
        assert_eq!(described.escaped_functions, BTreeSet::from([F]));
        assert!(described.open.contains(&FlowVar::Temp(1)));
    }

    #[test]
    fn spread_arguments_open_the_parameters_they_may_fill() {
        let spread = FlowCall {
            spread_from: Some(0),
            ..call(FlowVar::Local(0), vec![FlowVar::Local(1)])
        };
        let described = describe(
            &flows(
                vec![
                    copy(FlowVar::Local(0), FlowVar::Function(F)),
                    copy(FlowVar::Local(1), FlowVar::Prim),
                ],
                vec![spread],
            ),
            |_| true,
        );
        assert!(described.open.contains(&FlowVar::Param(F, 0)));
    }

    #[test]
    fn a_new_of_an_unknown_callee_escapes_its_object() {
        let construct = FlowCall {
            receiver: Some(FlowVar::Temp(5)),
            constructs: Some(OBJ),
            ..call(FlowVar::Open, Vec::new())
        };
        let rows = vec![FlowRow::Alloc {
            dst: FlowVar::Temp(5),
            object: FlowObject::Site(OBJ),
        }];
        let described = describe(&flows(rows, vec![construct]), |_| true);
        assert_eq!(described.escaped_objects, BTreeSet::from([Obj::Site(OBJ)]));
    }

    #[test]
    fn a_file_that_was_not_lowered_describes_nothing() {
        let mut unlowered = flows(
            vec![copy(FlowVar::Local(0), FlowVar::Function(F))],
            vec![call(FlowVar::Local(0), Vec::new())],
        );
        unlowered.lowered = false;
        assert_eq!(describe(&unlowered, |_| true), Described::default());
    }
}

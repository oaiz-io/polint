//! Which values of one lowered file its rows describe completely.
//!
//! A small inclusion solve over the rows of a single file. Every value the
//! rows cannot account for is the token `U`: what an import, a global, an
//! unknown call or the parameter of an escaped function may hold. A call site
//! whose callee set holds only functions of the file, and no `U`, is described
//! completely; the graph then names the rows' callee there.
//!
//! The solve is conservative in the same direction as the points-to solver:
//! a value that reaches a sink, an unknown callee or a slot the rows do not
//! track escapes, and code the rows do not describe may then read every slot
//! of an escaped object and call an escaped function with anything. The graph
//! copies [`Described::open`] from an open place and passes
//! [`Described::escaped_functions`] and [`Described::escaped_objects`] to a
//! sink, so the points-to solve is never more certain than this one.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::{FlowCall, FlowRow, FlowSpan, FlowVar, TsValueFlows};

/// A value of the solve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Token {
    /// A value the rows do not describe.
    Unknown,
    /// The function object of a lowered function that has a graph node.
    Function(FlowSpan),
    /// An object the rows allocate.
    Object(FlowSpan),
    /// A primitive value: its properties are the language's own, and calling
    /// it throws.
    Prim,
}

/// What the rows of one file describe.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Described {
    /// Call sites whose callee set the rows describe completely, with the
    /// largest parameter count among those callees: arguments are padded to
    /// it so every parameter a call fills is bound.
    pub(crate) named: BTreeMap<FlowSpan, u8>,
    /// Variables that may hold a value the rows do not describe.
    pub(crate) open: BTreeSet<FlowVar>,
    /// Functions that code the rows do not describe may call.
    pub(crate) escaped_functions: BTreeSet<FlowSpan>,
    /// Objects that code the rows do not describe may read and write.
    pub(crate) escaped_objects: BTreeSet<FlowSpan>,
}

/// Solves the rows of `flows`. `has_node` says whether the graph has a
/// function node for a span; a function without one is not a value the graph
/// can name, so it is `U` here.
pub(crate) fn describe(flows: &TsValueFlows, has_node: impl Fn(FlowSpan) -> bool) -> Described {
    if !flows.lowered {
        return Described::default();
    }
    let mut solve = Solve::new(flows, has_node);
    solve.run();
    solve.finish()
}

struct Solve<'a> {
    flows: &'a TsValueFlows,
    /// Lowered functions with a graph node, by span, with their parameter
    /// count.
    functions: BTreeMap<FlowSpan, u8>,
    index: HashMap<FlowVar, usize>,
    vars: Vec<Option<FlowVar>>,
    sets: Vec<BTreeSet<Token>>,
    slots: BTreeMap<(FlowSpan, Box<str>), usize>,
    stored: BTreeSet<(FlowSpan, Box<str>)>,
    escaped_functions: BTreeSet<FlowSpan>,
    escaped_objects: BTreeSet<FlowSpan>,
    changed: bool,
}

impl<'a> Solve<'a> {
    fn new(flows: &'a TsValueFlows, has_node: impl Fn(FlowSpan) -> bool) -> Self {
        let functions = flows
            .functions
            .iter()
            .filter(|function| has_node(function.span))
            .map(|function| (function.span, function.params))
            .collect();
        let mut solve = Self {
            flows,
            functions,
            index: HashMap::new(),
            vars: Vec::new(),
            sets: Vec::new(),
            slots: BTreeMap::new(),
            stored: BTreeSet::new(),
            escaped_functions: BTreeSet::new(),
            escaped_objects: BTreeSet::new(),
            changed: false,
        };
        solve.seed();
        solve
    }

    fn var(&mut self, var: FlowVar) -> usize {
        if let Some(index) = self.index.get(&var) {
            return *index;
        }
        let index = self.sets.len();
        self.index.insert(var, index);
        self.vars.push(Some(var));
        self.sets.push(BTreeSet::new());
        index
    }

    fn slot(&mut self, object: FlowSpan, field: &str) -> usize {
        if let Some(index) = self.slots.get(&(object, Box::from(field))) {
            return *index;
        }
        let index = self.sets.len();
        self.slots.insert((object, Box::from(field)), index);
        self.vars.push(None);
        self.sets.push(BTreeSet::new());
        index
    }

    fn add(&mut self, index: usize, token: Token) {
        if self.sets[index].insert(token) {
            self.changed = true;
        }
    }

    fn union(&mut self, dst: usize, src: usize) {
        if dst == src {
            return;
        }
        let missing = self.sets[src]
            .difference(&self.sets[dst])
            .copied()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            self.sets[dst].extend(missing);
            self.changed = true;
        }
    }

    /// Seeds the open place, every function object, and every variable that
    /// is read but that no row writes.
    fn seed(&mut self) {
        let open = self.var(FlowVar::Open);
        self.add(open, Token::Unknown);
        let prim = self.var(FlowVar::Prim);
        self.add(prim, Token::Prim);
        let mut written = BTreeSet::new();
        let mut read = BTreeSet::new();
        for row in &self.flows.rows {
            match row {
                FlowRow::Copy { dst, src } => {
                    written.insert(*dst);
                    read.insert(*src);
                }
                FlowRow::Alloc { dst, .. } => {
                    written.insert(*dst);
                }
                FlowRow::Load { dst, base, .. } => {
                    written.insert(*dst);
                    read.insert(*base);
                }
                FlowRow::Store { base, src, .. } => {
                    read.insert(*base);
                    read.insert(*src);
                }
            }
        }
        for call in &self.flows.calls {
            written.insert(call.result);
            read.insert(call.callee);
            read.extend(call.receiver);
            read.extend(call.arguments.iter().copied());
        }
        for var in read.iter().chain(&written) {
            let index = self.var(*var);
            if let FlowVar::Function(span) = var {
                let token = if self.functions.contains_key(span) {
                    Token::Function(*span)
                } else {
                    Token::Unknown
                };
                self.add(index, token);
            }
        }
        for var in read.difference(&written) {
            if matches!(var, FlowVar::Local(_) | FlowVar::Temp(_)) {
                let index = self.var(*var);
                self.add(index, Token::Unknown);
            }
        }
        // A lowered function without a graph node is not named anywhere, so
        // its parameters and `this` may hold anything and its return escapes.
        for function in &self.flows.functions {
            if !self.functions.contains_key(&function.span) {
                self.open_signature(function.span, function.params);
            }
        }
    }

    fn open_signature(&mut self, span: FlowSpan, params: u8) {
        for position in 0..params {
            let param = self.var(FlowVar::Param(span, position));
            self.add(param, Token::Unknown);
        }
        let this = self.var(FlowVar::This(span));
        self.add(this, Token::Unknown);
        let ret = self.var(FlowVar::Return(span));
        let sink = self.var(FlowVar::Sink);
        self.union(sink, ret);
    }

    fn run(&mut self) {
        loop {
            self.inner_fixpoint();
            self.changed = false;
            self.unstored_loads();
            self.unknown_constructors();
            if !self.changed {
                break;
            }
        }
    }

    fn inner_fixpoint(&mut self) {
        loop {
            self.changed = false;
            for row in &self.flows.rows {
                self.apply_row(row);
            }
            for call in &self.flows.calls {
                self.apply_call(call);
            }
            self.apply_escapes();
            if !self.changed {
                break;
            }
        }
    }

    fn apply_row(&mut self, row: &FlowRow) {
        match row {
            FlowRow::Copy { dst, src } => {
                let (dst, src) = (self.var(*dst), self.var(*src));
                self.union(dst, src);
            }
            FlowRow::Alloc { dst, object } => {
                let dst = self.var(*dst);
                self.add(dst, Token::Object(*object));
            }
            FlowRow::Load { dst, base, field } => {
                let (dst, base) = (self.var(*dst), self.var(*base));
                for token in self.sets[base].clone() {
                    match token {
                        Token::Object(object) if !self.escaped_objects.contains(&object) => {
                            let slot = self.slot(object, field);
                            self.union(dst, slot);
                        }
                        Token::Function(function) => {
                            // A function's own properties are not tracked,
                            // and `f.call` or `f.bind` hand it to code the
                            // rows do not describe.
                            self.add(dst, Token::Unknown);
                            if self.escaped_functions.insert(function) {
                                self.changed = true;
                            }
                        }
                        Token::Unknown | Token::Object(_) | Token::Prim => {
                            self.add(dst, Token::Unknown);
                        }
                    }
                }
            }
            FlowRow::Store { base, field, src } => {
                let (base, src) = (self.var(*base), self.var(*src));
                let sink = self.var(FlowVar::Sink);
                for token in self.sets[base].clone() {
                    match token {
                        Token::Object(object) if !self.escaped_objects.contains(&object) => {
                            let slot = self.slot(object, field);
                            self.union(slot, src);
                            self.stored.insert((object, field.clone()));
                        }
                        Token::Unknown | Token::Function(_) | Token::Object(_) | Token::Prim => {
                            self.union(sink, src);
                        }
                    }
                }
            }
        }
    }

    fn apply_call(&mut self, call: &FlowCall) {
        let callee = self.var(call.callee);
        let result = self.var(call.result);
        let sink = self.var(FlowVar::Sink);
        let receiver = call.receiver.map(|receiver| self.var(receiver));
        let arguments = call
            .arguments
            .iter()
            .map(|argument| self.var(*argument))
            .collect::<Vec<_>>();
        let bound = call
            .spread_from
            .map_or(arguments.len(), |spread| spread.min(arguments.len()));
        for token in self.sets[callee].clone() {
            let function = match token {
                Token::Function(function) => function,
                // Calling a primitive throws: nothing flows.
                Token::Prim => continue,
                Token::Unknown | Token::Object(_) => {
                    for value in receiver.iter().chain(&arguments) {
                        self.union(sink, *value);
                    }
                    self.add(result, Token::Unknown);
                    continue;
                }
            };
            let params = self.functions.get(&function).copied().unwrap_or(0);
            let this = self.var(FlowVar::This(function));
            match receiver {
                Some(receiver) => self.union(this, receiver),
                None => self.add(this, Token::Unknown),
            }
            for (position, argument) in arguments.iter().enumerate() {
                match u8::try_from(position) {
                    Ok(position) if usize::from(position) < bound && position < params => {
                        let param = self.var(FlowVar::Param(function, position));
                        self.union(param, *argument);
                    }
                    _ => self.union(sink, *argument),
                }
            }
            if call.spread_from.is_some() {
                for position in u8::try_from(bound).unwrap_or(u8::MAX)..params {
                    let param = self.var(FlowVar::Param(function, position));
                    self.add(param, Token::Unknown);
                }
            }
            let ret = self.var(FlowVar::Return(function));
            self.union(result, ret);
        }
    }

    /// Escapes what reaches the sink, and opens what escaped values expose.
    fn apply_escapes(&mut self) {
        let sink = self.var(FlowVar::Sink);
        for token in self.sets[sink].clone() {
            let added = match token {
                Token::Function(function) => self.escaped_functions.insert(function),
                Token::Object(object) => self.escaped_objects.insert(object),
                Token::Unknown | Token::Prim => false,
            };
            self.changed |= added;
        }
        for function in self.escaped_functions.clone() {
            let params = self.functions.get(&function).copied().unwrap_or(0);
            self.open_signature(function, params);
        }
        let escaped_slots = self
            .slots
            .iter()
            .filter(|((object, _), _)| self.escaped_objects.contains(object))
            .map(|(_, slot)| *slot)
            .collect::<Vec<_>>();
        for slot in escaped_slots {
            self.union(sink, slot);
        }
    }

    /// A field no row stores may still be found on the prototype, which the
    /// rows do not describe.
    fn unstored_loads(&mut self) {
        for row in &self.flows.rows {
            let FlowRow::Load { dst, base, field } = row else {
                continue;
            };
            let (dst, base) = (self.var(*dst), self.var(*base));
            let unstored = self.sets[base].iter().any(|token| {
                matches!(token, Token::Object(object)
                    if !self.escaped_objects.contains(object)
                        && !self.stored.contains(&(*object, field.clone())))
            });
            if unstored {
                self.add(dst, Token::Unknown);
            }
        }
    }

    /// An object a `new` allocates for a callee the rows do not describe gets
    /// its prototype, and so its methods, from that callee.
    fn unknown_constructors(&mut self) {
        for call in &self.flows.calls {
            let Some(object) = call.constructs else {
                continue;
            };
            let callee = self.var(call.callee);
            let unknown = self.sets[callee].iter().any(|token| match token {
                Token::Function(function) => self.escaped_functions.contains(function),
                Token::Unknown | Token::Object(_) => true,
                Token::Prim => false,
            });
            if unknown && self.escaped_objects.insert(object) {
                self.changed = true;
            }
        }
    }

    /// The largest parameter count of a callee set the rows describe: one
    /// that holds a lowered function and otherwise only primitives.
    fn described_pad(&self, set: &BTreeSet<Token>) -> Option<u8> {
        let mut pad = None::<u8>;
        for token in set {
            match token {
                Token::Function(function) => {
                    let params = self.functions.get(function).copied().unwrap_or(0);
                    pad = Some(pad.map_or(params, |held| held.max(params)));
                }
                Token::Prim => {}
                Token::Unknown | Token::Object(_) => return None,
            }
        }
        pad
    }

    fn finish(self) -> Described {
        // A site lowered twice is named only when every lowering is described.
        let mut sites = BTreeMap::<FlowSpan, Option<u8>>::new();
        for call in &self.flows.calls {
            let pad = self
                .index
                .get(&call.callee)
                .and_then(|index| self.described_pad(&self.sets[*index]));
            let entry = sites.entry(call.site).or_insert(Some(0));
            *entry = entry.zip(pad).map(|(held, pad)| held.max(pad));
        }
        let named = sites
            .into_iter()
            .filter_map(|(site, pad)| Some((site, pad?)))
            .collect();
        let open = self
            .vars
            .iter()
            .zip(&self.sets)
            .filter_map(|(var, set)| {
                let var = (*var)?;
                let tracked = matches!(
                    var,
                    FlowVar::Local(_)
                        | FlowVar::Temp(_)
                        | FlowVar::Param(..)
                        | FlowVar::This(_)
                        | FlowVar::Return(_)
                );
                (tracked && set.contains(&Token::Unknown)).then_some(var)
            })
            .collect();
        Described {
            named,
            open,
            escaped_functions: self.escaped_functions,
            escaped_objects: self.escaped_objects,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ts::constraints::LoweredFunction;

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
                object: OBJ,
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
                object: OBJ,
            },
            FlowRow::Store {
                base: FlowVar::Local(0),
                field: "exact:run".into(),
                src: FlowVar::Function(F),
            },
            copy(FlowVar::Sink, FlowVar::Local(0)),
        ];
        let described = describe(&flows(rows, Vec::new()), |_| true);
        assert_eq!(described.escaped_objects, BTreeSet::from([OBJ]));
        assert_eq!(described.escaped_functions, BTreeSet::from([F]));
        assert!(described.open.contains(&FlowVar::Param(F, 0)));
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
            object: OBJ,
        }];
        let described = describe(&flows(rows, vec![construct]), |_| true);
        assert_eq!(described.escaped_objects, BTreeSet::from([OBJ]));
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

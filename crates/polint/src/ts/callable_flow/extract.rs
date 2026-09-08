use std::collections::BTreeMap;

use oxc_ast::ast::{
    Argument, ArrayExpressionElement, AssignmentTarget, BinaryOperator, BindingPattern,
    CallExpression, Class, ClassElement, Declaration, ExportDefaultDeclarationKind, Expression,
    ForStatementLeft, FunctionBody, ImportDeclarationSpecifier, LogicalOperator, MethodDefinition,
    MethodDefinitionKind, ModuleExportName, ObjectProperty, ObjectPropertyKind, Program,
    PropertyKey, PropertyKind, Statement, VariableDeclaration, VariableDeclarator,
};
use oxc_span::GetSpan;

use crate::analysis_api::{FunctionFact, ResolutionStatus, SourceFile, TS_JS_MODULE_FUNCTION_NAME};
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::calls::facts::{CallCallee, CallSiteFact};
use crate::internal_core::{FileId, FunctionId};

use super::{TsCallableFlow, TsCallableFlowKind};

/// Maximum bounded rounds for the cross-module export-summary fixpoint. Each
/// round lets a module's `require`/`import` targets observe the previous
/// round's summaries, so re-export chains such as
/// `module.exports = require('./lib/foo')` converge after a few rounds. The
/// bound keeps deep dependency graphs (e.g. express) finite.
const MAX_MODULE_SUMMARY_ROUNDS: usize = 4;

pub(in crate::ts) fn collect_callable_flows<'ast>(
    db: &impl AnalysisHost,
    programs: &BTreeMap<FileId, &'ast Program<'ast>>,
) -> Vec<TsCallableFlow> {
    let mut rows = Vec::new();
    let sites = db.call_sites();
    let sites_by_file = sites_by_file(sites);
    let functions_by_file_start = functions_by_file_start(db);
    let resolution_map = module_resolution_map(db);
    let module_summaries =
        compute_module_export_summaries(db, programs, &functions_by_file_start, &resolution_map);
    let function_returns = compute_function_return_summaries(
        db,
        programs,
        &functions_by_file_start,
        &resolution_map,
        &module_summaries,
    );

    for file in db
        .files()
        .iter()
        .filter(|file| file.language.is_ts_family())
    {
        let Some(file_sites) = sites_by_file.get(&file.id) else {
            continue;
        };
        let Some(module) = module_function(db, file) else {
            continue;
        };
        let Some(&program) = programs.get(&file.id) else {
            continue;
        };
        let mut collector = TsCallableFlowCollector {
            file,
            module,
            sites: file_sites,
            functions_by_start: &functions_by_file_start,
            resolution_map: &resolution_map,
            module_summaries: &module_summaries,
            function_returns: &function_returns,
            function_declarations: BTreeMap::new(),
            function_flows_by_id: BTreeMap::new(),
            classes: BTreeMap::new(),
            class_expressions: Vec::new(),
            exports: ModuleExportSummary::default(),
            export_aliases: std::collections::BTreeSet::new(),
            caller_override: None,
            this_method_walk: false,
            current_super: None,
            invocation_depth: 0,
            iterator_next_ordinals: BTreeMap::new(),
            iterator_next_starts: BTreeMap::new(),
            rows: Vec::new(),
        };
        collector.collect_program(program);
        rows.extend(collector.rows);
    }

    rows.sort();
    rows.dedup();
    rows
}

struct TsCallableFlowCollector<'db, 'ast, 'env> {
    file: &'db SourceFile,
    module: &'db FunctionFact,
    sites: &'db [&'db CallSiteFact],
    functions_by_start: &'db BTreeMap<(FileId, u32), Vec<&'db FunctionFact>>,
    /// `(importer file, specifier) -> resolved target file`, reusing the kernel
    /// module graph's `require`/`import` resolution.
    resolution_map: &'env BTreeMap<(FileId, String), FileId>,
    /// Converged per-file CommonJS/ESM export summaries used to seed
    /// `require(...)` / `import` results into the current file's value flow.
    module_summaries: &'env BTreeMap<FileId, ModuleExportSummary>,
    /// Per-function return-value summaries (the object/callable shape a function
    /// returns), keyed by `FunctionId`, computed across all files. Lets a call to
    /// a function defined in another file seed its result — e.g. `express()`
    /// returns the `app` object that `createApplication` builds via `mixin`.
    /// Empty during the summary pre-passes; populated for the main resolution pass.
    function_returns: &'env BTreeMap<FunctionId, ModuleExportSummary>,
    function_declarations: BTreeMap<String, FunctionFlow<'db, 'ast>>,
    function_flows_by_id: BTreeMap<FunctionId, FunctionFlow<'db, 'ast>>,
    classes: BTreeMap<String, ClassTargets>,
    /// Class **expressions** (named, anonymous, or returned from a function),
    /// each keyed by a span-derived synthetic name registered in `classes`.
    /// Their method/constructor/static bodies are walked with `this`/`super`
    /// bound just like top-level class declarations.
    class_expressions: Vec<(String, &'ast Class<'ast>)>,
    /// This file's own exports, accumulated while walking module-level
    /// statements (only meaningful during the summary pre-pass). During the
    /// summary pre-pass `sites` is empty, so the `emit_*` helpers naturally
    /// produce no rows and only `exports` is harvested.
    exports: ModuleExportSummary,
    /// Local variable names aliasing the module's export object
    /// (`var app = exports = module.exports = {}`). Property writes to these
    /// (`app.listen = fn`, `app[method] = fn`) build the export object, so after
    /// the module body walk their `env.objects` entry is folded into `exports`.
    export_aliases: std::collections::BTreeSet<String>,
    /// When set, emitted edges use this function as the caller instead of the
    /// call site's enclosing function. Class construction owns constructor-body
    /// and field-initializer calls,
    /// so the class-body walk sets this to the class function fact while walking
    /// the constructor.
    caller_override: Option<FunctionId>,
    /// True during speculative export-object method walks with a modeled `this`.
    /// These flows retain a separate receiver-model provenance label.
    this_method_walk: bool,
    /// The super-class instance/static member objects in scope while walking a
    /// class method body, used to resolve `super.m()` / `super.s()` / `super.f`.
    current_super: Option<ObjectTargets>,
    /// Nesting depth of function-return-value invocation walks. Bounds recursion
    /// for self-/mutually-returning functions (`function f() { return f(); }`),
    /// which would otherwise loop forever because the per-call `depth` resets to 0
    /// when a return statement re-enters `callable_return_targets_from_expression`.
    invocation_depth: usize,
    /// Program-global positional index of every `receiver.next()` call: maps the
    /// `.next()` call's `span.start` to its 0-based ordinal among `.next()` calls
    /// on the same receiver name (in source order). Used to sequence generator
    /// `.next()` results positionally instead of lumping every yield onto every
    /// `.next()`.
    iterator_next_ordinals: BTreeMap<u32, usize>,
    /// Sorted `.next()` call `span.start`s per receiver name, so a `for-of` over
    /// an iterator can skip the yields already consumed by preceding `.next()`s.
    iterator_next_starts: BTreeMap<String, Vec<u32>>,
    rows: Vec<TsCallableFlow>,
}

/// CommonJS/ESM export shape of a single module: the callable values reachable
/// through `module.exports = fn` (or `export default`) plus the property object
/// reachable through `exports.foo = ...` / `module.exports = { foo }`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ModuleExportSummary {
    object: ObjectTargets,
    callables: CollectionTargets,
}

impl ModuleExportSummary {
    fn is_empty(&self) -> bool {
        self.object.is_empty() && self.callables.is_empty()
    }
}

#[derive(Clone)]
struct FunctionFlow<'db, 'ast> {
    function: &'db FunctionFact,
    body: &'ast FunctionBody<'ast>,
    expression_body: Option<&'ast Expression<'ast>>,
    params: Vec<ParamPattern>,
    rest: Option<ParamPattern>,
    generator: bool,
}

#[derive(Clone, Debug)]
enum ParamPattern {
    Binding(String),
    Array {
        elements: Vec<Option<ParamPattern>>,
        rest: Option<Box<ParamPattern>>,
    },
    Object {
        properties: Vec<(String, ParamPattern)>,
        rest: Option<Box<ParamPattern>>,
    },
}

/// Join import specifiers to files through the canonical module graph. Unresolved
/// or external targets cannot seed a local callable summary.
fn module_resolution_map(db: &impl AnalysisHost) -> BTreeMap<(FileId, String), FileId> {
    let imports = db
        .imports()
        .iter()
        .map(|import| (import.id, import))
        .collect::<BTreeMap<_, _>>();
    let files_by_node = db
        .module_nodes()
        .iter()
        .filter_map(|node| node.file.map(|file| (node.id, file)))
        .collect::<BTreeMap<_, _>>();
    db.resolved_imports()
        .iter()
        .filter_map(|resolution| {
            if resolution.status != ResolutionStatus::Resolved {
                return None;
            }
            let import = imports.get(&resolution.import)?;
            let target = *files_by_node.get(&resolution.target_node?)?;
            (target != import.file).then(|| ((import.file, import.path.clone()), target))
        })
        .collect()
}

/// Compute each module's export summary via a bounded fixpoint. Round 0 sees no
/// upstream summaries (so only locally-defined exports resolve); each later
/// round lets `require`/`import` results observe the previous round's
/// summaries, converging re-export chains.
fn compute_module_export_summaries<'ast>(
    db: &impl AnalysisHost,
    programs: &BTreeMap<FileId, &'ast Program<'ast>>,
    functions_by_file_start: &BTreeMap<(FileId, u32), Vec<&FunctionFact>>,
    resolution_map: &BTreeMap<(FileId, String), FileId>,
) -> BTreeMap<FileId, ModuleExportSummary> {
    let mut summaries: BTreeMap<FileId, ModuleExportSummary> = BTreeMap::new();
    if resolution_map.is_empty() {
        // No cross-module imports were resolved, so there is nothing for the
        // summary fixpoint to feed; skip the (expensive) pre-pass entirely.
        return summaries;
    }
    // Function-return seeding is not active while building export summaries.
    let empty_returns: BTreeMap<FunctionId, ModuleExportSummary> = BTreeMap::new();
    for _round in 0..MAX_MODULE_SUMMARY_ROUNDS {
        let mut next: BTreeMap<FileId, ModuleExportSummary> = BTreeMap::new();
        for file in db
            .files()
            .iter()
            .filter(|file| file.language.is_ts_family())
        {
            let Some(module) = module_function(db, file) else {
                continue;
            };
            let Some(&program) = programs.get(&file.id) else {
                continue;
            };
            let mut collector = TsCallableFlowCollector {
                file,
                module,
                sites: &[],
                functions_by_start: functions_by_file_start,
                resolution_map,
                module_summaries: &summaries,
                function_returns: &empty_returns,
                function_declarations: BTreeMap::new(),
                function_flows_by_id: BTreeMap::new(),
                classes: BTreeMap::new(),
                class_expressions: Vec::new(),
                exports: ModuleExportSummary::default(),
                export_aliases: std::collections::BTreeSet::new(),
                caller_override: None,
                this_method_walk: false,
                current_super: None,
                invocation_depth: 0,
                iterator_next_ordinals: BTreeMap::new(),
                iterator_next_starts: BTreeMap::new(),
                rows: Vec::new(),
            };
            collector.collect_program(program);
            if !collector.exports.is_empty() {
                next.insert(file.id, collector.exports);
            }
        }
        if next == summaries {
            break;
        }
        summaries = next;
    }
    summaries
}

/// Compute each function's return-value summary (object/callable shape it
/// returns), keyed by `FunctionId`, against the converged module summaries. A
/// single pass suffices for the chains we model (`express()` →
/// `createApplication` → returns the `mixin`-assembled `app`); return chains
/// across summarized functions are not threaded (the map is empty while
/// computing). Skipped entirely when no cross-module imports resolve.
fn compute_function_return_summaries<'ast>(
    db: &impl AnalysisHost,
    programs: &BTreeMap<FileId, &'ast Program<'ast>>,
    functions_by_file_start: &BTreeMap<(FileId, u32), Vec<&FunctionFact>>,
    resolution_map: &BTreeMap<(FileId, String), FileId>,
    module_summaries: &BTreeMap<FileId, ModuleExportSummary>,
) -> BTreeMap<FunctionId, ModuleExportSummary> {
    let mut returns: BTreeMap<FunctionId, ModuleExportSummary> = BTreeMap::new();
    if resolution_map.is_empty() {
        return returns;
    }
    let empty_returns = BTreeMap::new();
    for file in db
        .files()
        .iter()
        .filter(|file| file.language.is_ts_family())
    {
        let Some(module) = module_function(db, file) else {
            continue;
        };
        let Some(&program) = programs.get(&file.id) else {
            continue;
        };
        let mut collector = TsCallableFlowCollector {
            file,
            module,
            sites: &[],
            functions_by_start: functions_by_file_start,
            resolution_map,
            module_summaries,
            function_returns: &empty_returns,
            function_declarations: BTreeMap::new(),
            function_flows_by_id: BTreeMap::new(),
            classes: BTreeMap::new(),
            class_expressions: Vec::new(),
            exports: ModuleExportSummary::default(),
            export_aliases: std::collections::BTreeSet::new(),
            caller_override: None,
            this_method_walk: false,
            current_super: None,
            invocation_depth: 0,
            iterator_next_ordinals: BTreeMap::new(),
            iterator_next_starts: BTreeMap::new(),
            rows: Vec::new(),
        };
        collector.collect_return_summaries(program, &mut returns);
    }
    returns
}

impl<'db, 'ast, 'env> TsCallableFlowCollector<'db, 'ast, 'env> {
    fn collect_program(&mut self, program: &'ast Program<'ast>) {
        let env = self.build_module_env(program);
        // Resolve intra-method calls (`this.foo()`, `super.m()`, `this.#bar()`)
        // with `this`/`super` bound, after the module env is fully populated.
        self.collect_class_body_call_flows(program, &env);
        // Methods attached to the module's export object (`app.use = function(){
        // … this.lazyrouter() … }`) dispatch `this` to that object. Walk their
        // bodies with `this` bound, so the framework's internal `this.X()` calls
        // resolve once the object's entry methods become reachable (Express).
        self.collect_export_object_method_flows(&env);
        // Prototype-based OOP (`IPv4.prototype.toString = function(){ … this.x() …
        // }`) is the same shape for function-constructor classes that the class-body
        // walk does not cover. Walk those instance-method bodies with `this` bound.
        self.collect_prototype_method_this_flows(program, &env);
    }

    /// Walk the bodies of functions attached to an export-alias object with `this`
    /// bound to that object. `application.js` defines `app.use`, `app.listen`, the
    /// verb handler, etc. as `app.X = function(){ … this.lazyrouter() … }`; those
    /// `this.Y()` calls resolve only with `this` = the (fully built) `app` object.
    /// Attribution stays with each method (no `caller_override`). This models
    /// the same receiver binding as `collect_class_body_call_flows` for plain
    /// object methods.
    fn collect_export_object_method_flows(&mut self, env: &FlowEnv) {
        // Preserve the speculative receiver model in each flow's provenance.
        let saved_this_walk = self.this_method_walk;
        self.this_method_walk = true;
        let aliases: Vec<String> = self.export_aliases.iter().cloned().collect();
        for alias in aliases {
            let Some(object) = env.objects.get(&alias).cloned() else {
                continue;
            };
            // Every callable attached to the object (explicit properties + the
            // dynamic verb wildcard) is a candidate method body.
            let mut method_ids: Vec<FunctionId> = object
                .properties
                .values()
                .flatten()
                .copied()
                .chain(object.wildcard.iter().copied())
                .collect();
            method_ids.sort();
            method_ids.dedup();
            for method_id in method_ids {
                self.walk_method_body_with_this(method_id, &object, env);
            }
        }
        self.this_method_walk = saved_this_walk;
    }

    /// Walk the bodies of prototype methods of function-constructor classes
    /// (`function IPv4(){}; IPv4.prototype.toString = function(){ … this.x() … }`)
    /// with `this` bound to the instance object. This is the class-body walk for
    /// classes built the pre-`class` way — those entries in `self.classes` have no
    /// `Class` AST node, so `collect_class_body_call_flows` skips them. Flows
    /// retain the heuristic receiver-model label.
    fn collect_prototype_method_this_flows(
        &mut self,
        program: &'ast Program<'ast>,
        module_env: &FlowEnv,
    ) {
        // Class keys already walked with `this` bound as real `class` declarations
        // or expressions — skip them to avoid double-walking.
        let mut walked: std::collections::BTreeSet<String> = self
            .class_expressions
            .iter()
            .map(|(key, _)| key.clone())
            .collect();
        for statement in &program.body {
            if let Statement::ClassDeclaration(class) = statement
                && let Some(id) = class.id.as_ref()
            {
                walked.insert(id.name.to_string());
            }
        }
        let keys: Vec<String> = self
            .classes
            .keys()
            .filter(|key| !walked.contains(*key))
            .cloned()
            .collect();
        let saved_this_walk = self.this_method_walk;
        self.this_method_walk = true;
        for key in keys {
            let Some(instance) = self.class_instance_targets(&key, &[], module_env) else {
                continue;
            };
            let mut method_ids: Vec<FunctionId> = instance
                .properties
                .values()
                .flatten()
                .copied()
                .chain(instance.wildcard.iter().copied())
                .collect();
            method_ids.sort();
            method_ids.dedup();
            for method_id in method_ids {
                self.walk_method_body_with_this(method_id, &instance, module_env);
            }
        }
        self.this_method_walk = saved_this_walk;
    }

    /// Walk one method body with `this` bound to `this_object` (caller stays the
    /// method). Shared by the export-object and prototype-method `this`-walks.
    fn walk_method_body_with_this(
        &mut self,
        method_id: FunctionId,
        this_object: &ObjectTargets,
        module_env: &FlowEnv,
    ) {
        let Some(flow) = self.function_flows_by_id.get(&method_id).cloned() else {
            return;
        };
        let mut method_env = module_env.clone();
        method_env.this_object = this_object.clone();
        // The method's parameters shadow any module binding of the same name;
        // their values are unknown, so drop the inherited binding.
        for param in &flow.params {
            if let ParamPattern::Binding(name) = param {
                method_env.bindings.remove(name);
                method_env.objects.remove(name);
            }
        }
        for statement in &flow.body.statements {
            self.collect_statement(statement, flow.function, &mut method_env);
        }
    }

    /// `Receiver` while walking export-object method bodies speculatively,
    /// else `Token`. See `this_method_walk`.
    fn value_flow_kind(&self) -> TsCallableFlowKind {
        if self.this_method_walk {
            TsCallableFlowKind::Receiver
        } else {
            TsCallableFlowKind::Token
        }
    }

    /// Index inventories, declarations, and class tables, then walk the module
    /// body to build the top-level `FlowEnv`. Returns the populated env so both
    /// the main resolution pass (which then walks class bodies) and the
    /// return-summary pre-pass (which then evaluates each function's return) can
    /// reuse the identical setup.
    fn build_module_env(&mut self, program: &'ast Program<'ast>) -> FlowEnv {
        self.index_iterator_next_calls(program);
        self.collect_expression_function_flows(program);
        self.collect_function_declarations(program);
        self.collect_class_declarations(program);
        let mut env = FlowEnv::default();
        for name in self.classes.keys() {
            if let Some(targets) = self.class_static_targets(name) {
                env.objects.insert(name.clone(), targets);
            }
        }
        // ESM imports are hoisted: seed their bindings before walking the body.
        self.collect_esm_imports(program, &mut env);
        for statement in &program.body {
            self.collect_esm_export(statement, &mut env);
            self.collect_statement(statement, self.module, &mut env);
        }
        // An export-alias object (`var app = exports = module.exports = {}` then
        // `app.X = fn` / `app[m] = fn`) is the module's export shape. Fold it in
        // after the body walk so its property and wildcard members are visible to
        // requiring modules (Express's `application.js` builds `proto` this way).
        for name in &self.export_aliases {
            if let Some(object) = env.objects.get(name) {
                self.exports.object.merge(object.clone());
            }
        }
        env
    }

    /// Per-function return-value summaries for this file, evaluated against the
    /// module env (so `proto = require('./application')` and the `mixin` helper
    /// resolve). Keyed by `FunctionId` for cross-file seeding: `express()` in one
    /// file then resolves to the `app` object `createApplication` returns.
    fn collect_return_summaries(
        &mut self,
        program: &'ast Program<'ast>,
        returns: &mut BTreeMap<FunctionId, ModuleExportSummary>,
    ) {
        let env = self.build_module_env(program);
        let flows: Vec<FunctionFlow<'db, 'ast>> =
            self.function_declarations.values().cloned().collect();
        for flow in flows {
            let summary = self.function_return_summary(&flow, &env);
            if !summary.is_empty() {
                returns.insert(flow.function.id, summary);
            }
        }
    }

    /// Walk `flow`'s body with the module env as parent, then read the returned
    /// expression's object/callable shape. Bounded by `invocation_depth`.
    fn function_return_summary(
        &mut self,
        flow: &FunctionFlow<'db, 'ast>,
        module_env: &FlowEnv,
    ) -> ModuleExportSummary {
        if self.invocation_depth > 16 {
            return ModuleExportSummary::default();
        }
        let mut env = module_env.clone();
        self.invocation_depth += 1;
        let saved_override = self.caller_override.take();
        for statement in &flow.body.statements {
            self.collect_statement(statement, flow.function, &mut env);
        }
        let returned = returned_expression_from_statements(&flow.body.statements);
        let object = returned
            .and_then(|expression| self.object_targets_from_expression(expression, &env))
            .unwrap_or_default();
        let callables = returned
            .map(|expression| self.callable_targets_from_expression(expression, &env))
            .unwrap_or_default();
        self.caller_override = saved_override;
        self.invocation_depth -= 1;
        ModuleExportSummary { object, callables }
    }

    /// Program-global pre-scan assigning each `receiver.next()` call a 0-based
    /// ordinal among `.next()` calls on the same receiver name (by source
    /// position). Order-independent: spans are collected in any traversal order
    /// then sorted, so the ordinal is purely a function of source layout. Drives
    /// sequence-sensitive generator resolution (the k-th `.next()` → k-th yield).
    fn index_iterator_next_calls(&mut self, program: &'ast Program<'ast>) {
        let mut by_name: BTreeMap<String, Vec<u32>> = BTreeMap::new();
        for statement in &program.body {
            index_next_calls_in_statement(statement, &mut by_name);
        }
        for (name, mut starts) in by_name {
            starts.sort_unstable();
            starts.dedup();
            for (ordinal, start) in starts.iter().enumerate() {
                self.iterator_next_ordinals.insert(*start, ordinal);
            }
            self.iterator_next_starts.insert(name, starts);
        }
    }

    /// The ordinal of a `receiver.next()` call (its position among `.next()`
    /// calls on the same receiver), or `None` if it was not indexed.
    fn next_call_ordinal(&self, call: &CallExpression<'ast>) -> Option<usize> {
        self.iterator_next_ordinals.get(&call.span.start).copied()
    }

    /// How many `.next()` calls on `name` occur lexically before `before` —
    /// the number of yields a `for-of`/`for-await` over that iterator has
    /// already consumed.
    fn next_calls_consumed_before(&self, name: &str, before: u32) -> usize {
        self.iterator_next_starts
            .get(name)
            .map(|starts| starts.iter().filter(|start| **start < before).count())
            .unwrap_or(0)
    }

    /// Walk each class method and constructor body for call resolution with
    /// `this`/`super` in scope. Jelly treats every class function as reachable,
    /// and attributes constructor-body calls to the class node (via
    /// `caller_override`).
    fn collect_class_body_call_flows(
        &mut self,
        program: &'ast Program<'ast>,
        module_env: &FlowEnv,
    ) {
        // Seed top-level function declarations as callable bindings so direct
        // calls to them inside class bodies (e.g. `f1()` in a constructor or
        // static block) resolve and are attributed via `caller_override` to the
        // class node (Jelly attributes constructor/field/static-block calls to
        // the class). `function` declarations are otherwise absent from
        // `env.bindings` (unlike `const f = () => …`).
        let mut class_env = module_env.clone();
        for (name, flow) in &self.function_declarations {
            class_env
                .bindings
                .entry(name.clone())
                .or_insert_with(|| CollectionTargets {
                    keys: Vec::new(),
                    values: vec![flow.function.id],
                    object_values: Vec::new(),
                    ..Default::default()
                });
        }
        // Name -> class declaration AST, so a subclass's `super(arg)` can find the
        // superclass constructor to flow its arguments into.
        let class_asts: BTreeMap<String, &'ast Class<'ast>> = program
            .body
            .iter()
            .filter_map(|statement| match statement {
                Statement::ClassDeclaration(class) => class
                    .id
                    .as_ref()
                    .map(|id| (id.name.to_string(), class.as_ref())),
                _ => None,
            })
            .collect();
        for statement in &program.body {
            let Statement::ClassDeclaration(class) = statement else {
                continue;
            };
            let Some(name) = class.id.as_ref().map(|id| id.name.to_string()) else {
                continue;
            };
            self.collect_class_method_bodies(class, &name, &class_env, &class_asts);
        }
        // Class expressions (named, anonymous, or returned from a function) are
        // walked the same way, keyed by their span-derived synthetic name. Take
        // the registry (not read again) to avoid cloning it just to satisfy the
        // borrow checker while calling `&mut self`.
        let class_expressions = std::mem::take(&mut self.class_expressions);
        for (key, class) in &class_expressions {
            self.collect_class_method_bodies(class, key, &class_env, &class_asts);
        }
        self.class_expressions = class_expressions;
    }

    fn collect_class_method_bodies(
        &mut self,
        class: &'ast Class<'ast>,
        name: &str,
        module_env: &FlowEnv,
        class_asts: &BTreeMap<String, &'ast Class<'ast>>,
    ) {
        let instance = self
            .class_instance_targets(name, &[], module_env)
            .unwrap_or_default();
        let static_object = self.class_static_targets(name).unwrap_or_default();
        // Super-class member objects for `super.m()` / `super.s()` / `super.f`.
        let super_name = self
            .classes
            .get(name)
            .and_then(|class| class.super_name.clone());
        let super_instance = super_name
            .as_ref()
            .and_then(|super_name| self.class_instance_targets(super_name, &[], module_env));
        let super_static = super_name
            .as_ref()
            .and_then(|super_name| self.class_static_targets(super_name));
        // The class function fact owns constructor-body call attribution.
        let class_fact = self.function_for_span(class.span).map(|fact| fact.id);

        for element in &class.body.body {
            // A field initializer (`x = <expr>`) is its own function: walk the
            // initializer expression with `this`/`super` bound so calls in it
            // (`f1()`, `super.m()`) resolve and attribute to the field-init
            // function. Its body is an expression, not statements, so it is handled
            // separately from the statement-bodied elements below.
            if let ClassElement::PropertyDefinition(property) = element {
                if let Some(value) = &property.value {
                    // Key→property-end span (includes trailing `;`), matching the
                    // frontend FunctionFact + MIR body so `function_for_span` resolves.
                    let span = oxc_span::Span::new(property.key.span().start, property.span.end);
                    if let Some(owner) = self.function_for_span(span) {
                        let mut env = module_env.clone();
                        env.this_object = if property.r#static {
                            static_object.clone()
                        } else {
                            instance.clone()
                        };
                        env.objects.insert(name.to_string(), static_object.clone());
                        self.current_super = if property.r#static {
                            super_static.clone()
                        } else {
                            super_instance.clone()
                        };
                        self.collect_expression(value, owner, &mut env);
                        self.current_super = None;
                    }
                }
                continue;
            }
            let (statements, owner, is_static, is_constructor, params) = match element {
                ClassElement::MethodDefinition(method) => {
                    let Some(body) = method.value.body.as_deref() else {
                        continue;
                    };
                    let Some(owner) = self.owner_for_class_method(class, method) else {
                        continue;
                    };
                    let is_constructor = method.kind == MethodDefinitionKind::Constructor;
                    let params = method
                        .value
                        .params
                        .items
                        .iter()
                        .filter_map(|param| binding_identifier_name(&param.pattern))
                        .collect::<Vec<_>>();
                    (
                        &body.statements,
                        owner,
                        method.r#static,
                        is_constructor,
                        params,
                    )
                }
                ClassElement::StaticBlock(block) => {
                    let Some(owner) = self.function_for_span(block.span) else {
                        continue;
                    };
                    (&block.body, owner, true, false, Vec::new())
                }
                _ => continue,
            };
            let mut env = module_env.clone();
            env.this_object = if is_static {
                static_object.clone()
            } else {
                instance.clone()
            };
            // Bind the class name to its static object so `Class.staticM()` and
            // `Class.#priv()` resolve inside the body.
            env.objects.insert(name.to_string(), static_object.clone());
            // A parameter shadows any module-level binding of the same name (e.g.
            // a top-level function seeded into the class-body env). Its value is
            // unknown here, so drop the inherited binding rather than resolve the
            // call to the shadowed module symbol (a false positive).
            for param in &params {
                env.bindings.remove(param);
                env.class_bindings.remove(param);
                env.objects.remove(param);
            }
            self.current_super = if is_static {
                super_static.clone()
            } else {
                super_instance.clone()
            };
            // Constructor-body calls are attributed to the class node by Jelly.
            self.caller_override = if is_constructor { class_fact } else { None };
            for statement in statements {
                self.collect_statement(statement, owner, &mut env);
            }
            self.caller_override = None;
            self.current_super = None;
            // `super(arg)` flows the subclass's arguments into the superclass
            // constructor's parameters, so a param invoked there (`x()` in
            // `class A { constructor(x){ x() } }`) resolves to the passed value.
            if is_constructor {
                self.collect_super_constructor_arg_flows(
                    super_name.as_deref(),
                    statements,
                    super_instance.as_ref(),
                    module_env,
                    &env,
                    class_asts,
                );
            }
        }
    }

    /// Flow a `super(arg, …)` call's arguments into the superclass constructor's
    /// parameters and walk that constructor body, so an invoked parameter resolves
    /// to the passed value. Calls are attributed to the superclass node (Jelly
    /// attributes constructor-body calls to the class). Only literal/already-bound
    /// arguments resolve (e.g. `super(() => {})`); a `super(param)` whose value
    /// comes from the subclass's own unbound constructor param stays unresolved.
    fn collect_super_constructor_arg_flows(
        &mut self,
        super_name: Option<&str>,
        constructor_statements: &'ast [Statement<'ast>],
        super_instance: Option<&ObjectTargets>,
        module_env: &FlowEnv,
        caller_env: &FlowEnv,
        class_asts: &BTreeMap<String, &'ast Class<'ast>>,
    ) {
        let Some(super_name) = super_name else {
            return;
        };
        let Some(super_call) = find_super_call(constructor_statements) else {
            return;
        };
        let Some(super_class) = class_asts.get(super_name) else {
            return;
        };
        let Some(super_ctor) = constructor_method(super_class) else {
            return;
        };
        let Some(super_ctor_fact) = self.owner_for_class_method(super_class, super_ctor) else {
            return;
        };
        let Some(super_flow) = self.function_flows_by_id.get(&super_ctor_fact.id).cloned() else {
            return;
        };
        let super_node = self.classes.get(super_name).and_then(|c| c.constructor);
        let mut super_env = module_env.clone();
        if let Some(instance) = super_instance {
            super_env.this_object = instance.clone();
        }
        self.bind_call_arguments_to_flow(&super_flow, super_call, caller_env, &mut super_env);
        let saved_override = self.caller_override.take();
        let saved_super = self.current_super.take();
        self.caller_override = super_node;
        for statement in &super_flow.body.statements {
            self.collect_statement(statement, super_flow.function, &mut super_env);
        }
        self.caller_override = saved_override;
        self.current_super = saved_super;
    }

    fn collect_expression_function_flows(&mut self, program: &'ast Program<'ast>) {
        for statement in &program.body {
            self.collect_expression_function_flows_from_statement(statement);
        }
    }

    fn collect_expression_function_flows_from_statement(
        &mut self,
        statement: &'ast Statement<'ast>,
    ) {
        match statement {
            Statement::VariableDeclaration(variable) => {
                for declarator in &variable.declarations {
                    if let Some(init) = &declarator.init {
                        self.collect_expression_function_flows_from_expression(init);
                    }
                }
            }
            Statement::ExpressionStatement(statement) => {
                self.collect_expression_function_flows_from_expression(&statement.expression);
            }
            Statement::ReturnStatement(statement) => {
                if let Some(argument) = &statement.argument {
                    self.collect_expression_function_flows_from_expression(argument);
                }
            }
            Statement::BlockStatement(block) => {
                for statement in &block.body {
                    self.collect_expression_function_flows_from_statement(statement);
                }
            }
            Statement::IfStatement(statement) => {
                self.collect_expression_function_flows_from_expression(&statement.test);
                self.collect_expression_function_flows_from_statement(&statement.consequent);
                if let Some(alternate) = &statement.alternate {
                    self.collect_expression_function_flows_from_statement(alternate);
                }
            }
            Statement::ForOfStatement(statement) => {
                self.collect_expression_function_flows_from_expression(&statement.right);
                self.collect_expression_function_flows_from_statement(&statement.body);
            }
            Statement::FunctionDeclaration(function) => {
                if let Some(body) = function.body.as_deref() {
                    for statement in &body.statements {
                        self.collect_expression_function_flows_from_statement(statement);
                    }
                }
            }
            Statement::ClassDeclaration(class) => {
                self.collect_expression_function_flows_from_class(class);
            }
            _ => {}
        }
    }

    fn collect_expression_function_flows_from_expression(
        &mut self,
        expression: &'ast Expression<'ast>,
    ) {
        match expression {
            Expression::FunctionExpression(function) => {
                if let Some(body) = function.body.as_deref()
                    && let Some(function_fact) = self.function_for_expression(expression)
                {
                    let flow = FunctionFlow {
                        function: function_fact,
                        body,
                        expression_body: None,
                        params: function
                            .params
                            .items
                            .iter()
                            .map(|param| param_pattern_from_binding(&param.pattern))
                            .collect(),
                        rest: function
                            .params
                            .rest
                            .as_ref()
                            .map(|rest| param_pattern_from_binding(&rest.rest.argument)),
                        generator: false,
                    };
                    self.function_flows_by_id.insert(function_fact.id, flow);
                    for statement in &body.statements {
                        self.collect_expression_function_flows_from_statement(statement);
                    }
                }
            }
            Expression::ArrowFunctionExpression(function) => {
                if let Some(function_fact) = self.function_for_expression(expression) {
                    let flow = FunctionFlow {
                        function: function_fact,
                        body: &function.body,
                        expression_body: function.get_expression(),
                        params: function
                            .params
                            .items
                            .iter()
                            .map(|param| param_pattern_from_binding(&param.pattern))
                            .collect(),
                        rest: function
                            .params
                            .rest
                            .as_ref()
                            .map(|rest| param_pattern_from_binding(&rest.rest.argument)),
                        generator: false,
                    };
                    self.function_flows_by_id.insert(function_fact.id, flow);
                }
                if let Some(expression) = function.get_expression() {
                    self.collect_expression_function_flows_from_expression(expression);
                } else {
                    for statement in &function.body.statements {
                        self.collect_expression_function_flows_from_statement(statement);
                    }
                }
            }
            Expression::CallExpression(call) => {
                self.collect_expression_function_flows_from_expression(&call.callee);
                for argument in &call.arguments {
                    if let Some(argument) = argument_expression(argument) {
                        self.collect_expression_function_flows_from_expression(argument);
                    }
                }
            }
            Expression::NewExpression(expression) => {
                self.collect_expression_function_flows_from_expression(&expression.callee);
                for argument in &expression.arguments {
                    if let Some(argument) = argument_expression(argument) {
                        self.collect_expression_function_flows_from_expression(argument);
                    }
                }
            }
            Expression::AssignmentExpression(assignment) => {
                self.collect_expression_function_flows_from_expression(&assignment.right);
            }
            Expression::ArrayExpression(array) => {
                for element in &array.elements {
                    if let Some(expression) = array_element_expression(element) {
                        self.collect_expression_function_flows_from_expression(expression);
                    }
                }
            }
            Expression::ObjectExpression(object) => {
                for property in &object.properties {
                    match property {
                        ObjectPropertyKind::ObjectProperty(property) => {
                            self.collect_expression_function_flows_from_object_property(property);
                        }
                        ObjectPropertyKind::SpreadProperty(spread) => {
                            self.collect_expression_function_flows_from_expression(
                                &spread.argument,
                            );
                        }
                    }
                }
            }
            Expression::ClassExpression(class) => {
                self.register_class_expression(class);
                self.collect_expression_function_flows_from_class(class);
            }
            Expression::StaticMemberExpression(member) => {
                self.collect_expression_function_flows_from_expression(&member.object);
            }
            Expression::ComputedMemberExpression(member) => {
                self.collect_expression_function_flows_from_expression(&member.object);
                self.collect_expression_function_flows_from_expression(&member.expression);
            }
            Expression::AwaitExpression(expression) => {
                self.collect_expression_function_flows_from_expression(&expression.argument);
            }
            Expression::YieldExpression(expression) => {
                if let Some(argument) = &expression.argument {
                    self.collect_expression_function_flows_from_expression(argument);
                }
            }
            Expression::SequenceExpression(sequence) => {
                for expression in &sequence.expressions {
                    self.collect_expression_function_flows_from_expression(expression);
                }
            }
            Expression::ParenthesizedExpression(expression) => {
                self.collect_expression_function_flows_from_expression(&expression.expression);
            }
            Expression::ConditionalExpression(expression) => {
                self.collect_expression_function_flows_from_expression(&expression.consequent);
                self.collect_expression_function_flows_from_expression(&expression.alternate);
            }
            Expression::LogicalExpression(expression) => {
                self.collect_expression_function_flows_from_expression(&expression.left);
                self.collect_expression_function_flows_from_expression(&expression.right);
            }
            _ => {}
        }
    }

    fn collect_expression_function_flows_from_object_property(
        &mut self,
        property: &'ast ObjectProperty<'ast>,
    ) {
        if property.method
            && let Expression::FunctionExpression(function) = &property.value
        {
            if let Some(body) = function.body.as_deref()
                && let Some(function_fact) = self.function_for_object_property(property)
            {
                let flow = FunctionFlow {
                    function: function_fact,
                    body,
                    expression_body: None,
                    params: function
                        .params
                        .items
                        .iter()
                        .map(|param| param_pattern_from_binding(&param.pattern))
                        .collect(),
                    rest: function
                        .params
                        .rest
                        .as_ref()
                        .map(|rest| param_pattern_from_binding(&rest.rest.argument)),
                    generator: function.generator,
                };
                self.function_flows_by_id.insert(function_fact.id, flow);
                for statement in &body.statements {
                    self.collect_expression_function_flows_from_statement(statement);
                }
            }
            return;
        }

        self.collect_expression_function_flows_from_expression(&property.value);
    }

    fn collect_expression_function_flows_from_class(&mut self, class: &'ast Class<'ast>) {
        for element in &class.body.body {
            match element {
                ClassElement::MethodDefinition(method) => {
                    let Some(body) = method.value.body.as_deref() else {
                        continue;
                    };
                    let Some(function_fact) = self.owner_for_class_method(class, method) else {
                        continue;
                    };
                    let flow = FunctionFlow {
                        function: function_fact,
                        body,
                        expression_body: None,
                        params: method
                            .value
                            .params
                            .items
                            .iter()
                            .map(|param| param_pattern_from_binding(&param.pattern))
                            .collect(),
                        rest: method
                            .value
                            .params
                            .rest
                            .as_ref()
                            .map(|rest| param_pattern_from_binding(&rest.rest.argument)),
                        generator: method.value.generator,
                    };
                    self.function_flows_by_id.insert(function_fact.id, flow);
                    for statement in &body.statements {
                        self.collect_expression_function_flows_from_statement(statement);
                    }
                }
                ClassElement::PropertyDefinition(property) => {
                    if let Some(value) = &property.value {
                        self.collect_expression_function_flows_from_expression(value);
                    }
                }
                ClassElement::AccessorProperty(property) => {
                    if let Some(value) = &property.value {
                        self.collect_expression_function_flows_from_expression(value);
                    }
                }
                ClassElement::StaticBlock(block) => {
                    for statement in &block.body {
                        self.collect_expression_function_flows_from_statement(statement);
                    }
                }
                ClassElement::TSIndexSignature(_) => {}
            }
        }
    }

    fn collect_function_declarations(&mut self, program: &'ast Program<'ast>) {
        for statement in &program.body {
            let Statement::FunctionDeclaration(function) = statement else {
                continue;
            };
            let Some(name) = function.id.as_ref().map(|id| id.name.to_string()) else {
                continue;
            };
            let Some(body) = function.body.as_deref() else {
                continue;
            };
            if let Some(function_fact) = self.function_for_span(function.span) {
                let flow = FunctionFlow {
                    function: function_fact,
                    body,
                    expression_body: None,
                    params: function
                        .params
                        .items
                        .iter()
                        .map(|param| param_pattern_from_binding(&param.pattern))
                        .collect(),
                    rest: function
                        .params
                        .rest
                        .as_ref()
                        .map(|rest| param_pattern_from_binding(&rest.rest.argument)),
                    generator: function.generator,
                };
                self.function_flows_by_id
                    .insert(function_fact.id, flow.clone());
                self.function_declarations.insert(name.clone(), flow);
            }
            let mut targets = ClassTargets::default();
            self.collect_constructor_assignments_from_statements(
                &body.statements,
                &function
                    .params
                    .items
                    .iter()
                    .map(|param| param_pattern_from_binding(&param.pattern))
                    .collect::<Vec<_>>(),
                &mut targets,
            );
            if !targets.is_empty() {
                self.classes.insert(name, targets);
            }
        }
    }

    fn collect_class_declarations(&mut self, program: &'ast Program<'ast>) {
        for statement in &program.body {
            let Statement::ClassDeclaration(class) = statement else {
                continue;
            };
            let Some(name) = class.id.as_ref().map(|id| id.name.to_string()) else {
                continue;
            };
            let targets = self.class_targets(class);
            self.classes.insert(name, targets);
        }
    }

    /// Register a class **expression** under a span-derived synthetic key so its
    /// instance/static targets are resolvable and its bodies get walked with
    /// `this`/`super` bound (see `collect_class_body_call_flows`). Keyed by span
    /// (not by the class's own name) to avoid clobbering a same-named top-level
    /// class declaration; `new`/return flow binds variables to this key in A3.
    fn register_class_expression(&mut self, class: &'ast Class<'ast>) {
        let key = class_expression_key(class);
        if self.classes.contains_key(&key) {
            return;
        }
        let targets = self.class_targets(class);
        self.classes.insert(key.clone(), targets);
        self.class_expressions.push((key, class));
    }

    /// Resolve the class key a variable would hold for `var v = <expr>`, so a
    /// class flowed through a variable (returned from a function, aliased, or a
    /// direct class expression) resolves under `new v()` / `v.staticM()`.
    fn class_key_from_expression(
        &self,
        expression: &'ast Expression<'ast>,
        env: &FlowEnv,
    ) -> Option<String> {
        self.class_key_from_expression_with_depth(expression, env, 0)
    }

    fn class_key_from_expression_with_depth(
        &self,
        expression: &'ast Expression<'ast>,
        env: &FlowEnv,
        depth: usize,
    ) -> Option<String> {
        // Bound recursion: a function whose returned expression resolves back to
        // itself (`function f() { return (f()); }`) would otherwise loop forever.
        if depth > 8 {
            return None;
        }
        match expression {
            Expression::ClassExpression(class) => Some(class_expression_key(class)),
            Expression::ParenthesizedExpression(inner) => {
                self.class_key_from_expression_with_depth(&inner.expression, env, depth + 1)
            }
            Expression::Identifier(identifier) => {
                let name = identifier.name.as_str();
                env.class_bindings
                    .get(name)
                    .cloned()
                    .or_else(|| self.classes.contains_key(name).then(|| name.to_string()))
            }
            Expression::CallExpression(call) => {
                let name = callee_identifier(&call.callee)?;
                let flow = self.function_declarations.get(name)?;
                let returned = returned_expression_from_statements(&flow.body.statements)?;
                match returned {
                    Expression::ClassExpression(class) => Some(class_expression_key(class)),
                    Expression::Identifier(_) | Expression::ParenthesizedExpression(_) => {
                        self.class_key_from_expression_with_depth(returned, env, depth + 1)
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn class_targets(&self, class: &'ast Class<'ast>) -> ClassTargets {
        let mut targets = ClassTargets {
            super_name: class
                .super_class
                .as_ref()
                .and_then(|super_class| expression_identifier(super_class))
                .map(ToOwned::to_owned),
            constructor: self
                .function_for_span(class.span)
                .map(|function| function.id),
            ..ClassTargets::default()
        };
        for element in &class.body.body {
            match element {
                ClassElement::MethodDefinition(method) => {
                    if method.kind == MethodDefinitionKind::Constructor {
                        if let Some(body) = method.value.body.as_deref() {
                            let params = method
                                .value
                                .params
                                .items
                                .iter()
                                .map(|param| param_pattern_from_binding(&param.pattern))
                                .collect::<Vec<_>>();
                            self.collect_constructor_assignments_from_statements(
                                &body.statements,
                                &params,
                                &mut targets,
                            );
                        }
                        continue;
                    }
                    // Jelly resolves instance properties flow-insensitively: a
                    // `this.p = fn` in any non-static instance method flows to all
                    // instances (e.g. super5's `c.www()` where `www` is assigned in
                    // `m()`, never called). Harvest those function-valued assignments.
                    // Pass NO params: a method's parameters are not the constructor's,
                    // so `m(cb) { this.cb = cb }` must not register a
                    // `ConstructorAssignment::Param` that would later resolve against
                    // the `new C(arg)` arguments (a false-positive edge).
                    if !method.r#static
                        && let Some(body) = method.value.body.as_deref()
                    {
                        self.collect_constructor_assignments_from_statements(
                            &body.statements,
                            &[],
                            &mut targets,
                        );
                    }
                    let names = self.property_key_names(&method.key, method.computed, None);
                    if names.is_empty() {
                        continue;
                    };
                    let Some(function) = self.function_for_class_method(method) else {
                        continue;
                    };
                    for name in names {
                        let object = if method.r#static {
                            &mut targets.static_object
                        } else {
                            &mut targets.instance_object
                        };
                        match method.kind {
                            MethodDefinitionKind::Get => {
                                object.add_getter_target(name, function.id);
                            }
                            MethodDefinitionKind::Set => {
                                object.add_setter_target(name, function.id);
                            }
                            MethodDefinitionKind::Method => {
                                object.add_property_target(name, function.id);
                            }
                            MethodDefinitionKind::Constructor => {}
                        }
                    }
                }
                ClassElement::PropertyDefinition(property) => {
                    let names = self.property_key_names(&property.key, property.computed, None);
                    if names.is_empty() {
                        continue;
                    };
                    // `static g = super.f` aliases the field to the superclass's static
                    // member `f`, so `Sub.g()` dispatches to it. Resolve against the
                    // superclass's static targets here (before the mutable borrow of
                    // `targets` below). (super.js: `static g = super.f; B.g();`.)
                    let super_aliased: Option<Vec<FunctionId>> = property
                        .value
                        .as_ref()
                        .filter(|_| property.r#static)
                        .and_then(super_member_property)
                        .and_then(|prop| {
                            targets.super_name.as_ref().and_then(|super_name| {
                                self.class_static_targets(super_name)
                                    .and_then(|st| st.properties.get(prop).cloned())
                            })
                        });
                    let object = if property.r#static {
                        &mut targets.static_object
                    } else {
                        &mut targets.instance_object
                    };
                    let self_aliases = if property.r#static {
                        &mut targets.static_self_aliases
                    } else {
                        &mut targets.instance_self_aliases
                    };
                    if let Some(value) = &property.value {
                        // The value of `(f1(), function(){})` is its last sub-expression;
                        // unwrap parens/sequences so a field initialized to a comma-
                        // sequence resolves to the trailing function. (classes.js:
                        // `static staticProperty = (f1(), function(){}); C6.staticProperty()`.)
                        let resolved = field_value_expression(value);
                        for name in names {
                            if let Some(function) = self.function_for_expression(resolved) {
                                object.add_property_target(name, function.id);
                            } else if matches!(value, Expression::ThisExpression(_)) {
                                self_aliases.push(name);
                            } else if let Some(super_targets) = &super_aliased {
                                for target in super_targets {
                                    object.add_property_target(name.clone(), *target);
                                }
                            }
                        }
                    }
                }
                ClassElement::StaticBlock(block) => {
                    self.collect_static_assignments_from_statements(
                        &block.body,
                        &mut targets.static_object,
                    );
                }
                ClassElement::AccessorProperty(_) | ClassElement::TSIndexSignature(_) => {}
            }
        }
        targets
    }

    fn collect_constructor_assignments_from_statements(
        &self,
        statements: &'ast oxc_allocator::Vec<'ast, Statement<'ast>>,
        params: &[ParamPattern],
        targets: &mut ClassTargets,
    ) {
        for statement in statements {
            self.collect_constructor_assignment_from_statement(statement, params, targets);
        }
    }

    fn collect_constructor_assignment_from_statement(
        &self,
        statement: &'ast Statement<'ast>,
        params: &[ParamPattern],
        targets: &mut ClassTargets,
    ) {
        match statement {
            Statement::ExpressionStatement(statement) => {
                self.collect_constructor_assignment_from_expression(
                    &statement.expression,
                    params,
                    targets,
                );
            }
            Statement::BlockStatement(block) => {
                self.collect_constructor_assignments_from_statements(&block.body, params, targets);
            }
            Statement::IfStatement(statement) => {
                self.collect_constructor_assignment_from_statement(
                    &statement.consequent,
                    params,
                    targets,
                );
                if let Some(alternate) = &statement.alternate {
                    self.collect_constructor_assignment_from_statement(alternate, params, targets);
                }
            }
            _ => {}
        }
    }

    fn collect_constructor_assignment_from_expression(
        &self,
        expression: &'ast Expression<'ast>,
        params: &[ParamPattern],
        targets: &mut ClassTargets,
    ) {
        match expression {
            Expression::AssignmentExpression(assignment) => {
                // Static `this.X` plus computed `this["f" + "oo"]` (literal/concat
                // key, resolved without env since this is a structural pre-scan).
                let properties = this_assignment_static_or_concat_keys(&assignment.left);
                if properties.is_empty() {
                    return;
                }
                if let Some(function) = self.function_for_expression(&assignment.right) {
                    for property in properties {
                        targets
                            .instance_object
                            .add_property_target(property, function.id);
                    }
                    return;
                }
                if let Some(index) = param_index_for_expression(&assignment.right, params) {
                    for property in properties {
                        targets
                            .constructor_assignments
                            .push(ConstructorAssignment::Param { property, index });
                    }
                } else if matches!(&assignment.right, Expression::ThisExpression(_)) {
                    for property in properties {
                        targets.instance_self_aliases.push(property);
                    }
                }
            }
            Expression::SequenceExpression(sequence) => {
                for expression in &sequence.expressions {
                    self.collect_constructor_assignment_from_expression(
                        expression, params, targets,
                    );
                }
            }
            Expression::ParenthesizedExpression(expression) => self
                .collect_constructor_assignment_from_expression(
                    &expression.expression,
                    params,
                    targets,
                ),
            _ => {}
        }
    }

    fn collect_static_assignments_from_statements(
        &self,
        statements: &'ast oxc_allocator::Vec<'ast, Statement<'ast>>,
        object: &mut ObjectTargets,
    ) {
        for statement in statements {
            match statement {
                Statement::ExpressionStatement(statement) => {
                    self.collect_static_assignment_from_expression(&statement.expression, object);
                }
                Statement::BlockStatement(block) => {
                    self.collect_static_assignments_from_statements(&block.body, object);
                }
                _ => {}
            }
        }
    }

    fn collect_static_assignment_from_expression(
        &self,
        expression: &'ast Expression<'ast>,
        object: &mut ObjectTargets,
    ) {
        match expression {
            Expression::AssignmentExpression(assignment) => {
                let Some(property) = this_assignment_property(&assignment.left) else {
                    return;
                };
                if let Some(function) = self.function_for_expression(&assignment.right) {
                    object.add_property_target(property, function.id);
                }
            }
            Expression::SequenceExpression(sequence) => {
                for expression in &sequence.expressions {
                    self.collect_static_assignment_from_expression(expression, object);
                }
            }
            Expression::ParenthesizedExpression(expression) => {
                self.collect_static_assignment_from_expression(&expression.expression, object);
            }
            _ => {}
        }
    }

    fn class_instance_targets(
        &self,
        name: &str,
        arguments: &[&'ast Expression<'ast>],
        env: &FlowEnv,
    ) -> Option<ObjectTargets> {
        self.class_instance_targets_with_depth(name, arguments, env, 0)
    }

    fn class_instance_targets_with_depth(
        &self,
        name: &str,
        arguments: &[&'ast Expression<'ast>],
        env: &FlowEnv,
        depth: usize,
    ) -> Option<ObjectTargets> {
        if depth > 8 {
            return None;
        }
        let class = self.classes.get(name)?;
        let mut object = ObjectTargets::default();
        if let Some(super_name) = &class.super_name
            && let Some(super_object) =
                self.class_instance_targets_with_depth(super_name, arguments, env, depth + 1)
        {
            object.merge(super_object);
        }
        object.override_with(class.instance_object.clone());
        for assignment in &class.constructor_assignments {
            match assignment {
                ConstructorAssignment::Param { property, index } => {
                    if let Some(argument) = arguments.get(*index) {
                        for target in self
                            .callable_targets_from_expression(argument, env)
                            .all_targets()
                        {
                            object.add_property_target(property.clone(), target);
                        }
                    }
                }
            }
        }
        for alias in &class.instance_self_aliases {
            object.add_object_property(alias.clone(), object.clone());
        }
        Some(object)
    }

    fn class_static_targets(&self, name: &str) -> Option<ObjectTargets> {
        self.class_static_targets_with_depth(name, 0)
    }

    fn class_static_targets_with_depth(&self, name: &str, depth: usize) -> Option<ObjectTargets> {
        if depth > 8 {
            return None;
        }
        let class = self.classes.get(name)?;
        let mut object = ObjectTargets::default();
        if let Some(super_name) = &class.super_name
            && let Some(super_object) = self.class_static_targets_with_depth(super_name, depth + 1)
        {
            object.merge(super_object);
        }
        object.override_with(class.static_object.clone());
        for alias in &class.static_self_aliases {
            object.add_object_property(alias.clone(), object.clone());
        }
        Some(object)
    }

    /// Statically decide an `if` test that depends only on a known-absent
    /// parameter (`FlowEnv::absent`): `Some(true)`/`Some(false)` selects the live
    /// arm, `None` means undecidable (walk both arms — unchanged behavior). Kept
    /// to provably-safe shapes: `undefined` is nullish, falsy, and yields `false`
    /// for every relational comparison (NaN semantics).
    fn decide_branch(&self, test: &Expression<'ast>, env: &FlowEnv) -> Option<bool> {
        use oxc_ast::ast::UnaryOperator;
        let is_absent = |e: &Expression<'ast>| matches!(e, Expression::Identifier(id) if env.absent.contains(id.name.as_str()));
        let is_nullish = |e: &Expression<'ast>| {
            matches!(e, Expression::NullLiteral(_))
                || matches!(e, Expression::Identifier(id) if id.name == "undefined")
        };
        match test {
            Expression::ParenthesizedExpression(p) => self.decide_branch(&p.expression, env),
            // `!x` with x undefined -> true; bare `if (x)` with x undefined -> false.
            Expression::UnaryExpression(u)
                if u.operator == UnaryOperator::LogicalNot && is_absent(&u.argument) =>
            {
                Some(true)
            }
            Expression::Identifier(id) if env.absent.contains(id.name.as_str()) => Some(false),
            Expression::BinaryExpression(b) => {
                // `typeof x === 'undefined'` / `!== 'undefined'`.
                if let Expression::UnaryExpression(u) = &b.left
                    && u.operator == UnaryOperator::Typeof
                    && is_absent(&u.argument)
                    && let Expression::StringLiteral(s) = &b.right
                {
                    let undef = s.value == "undefined";
                    return match b.operator {
                        BinaryOperator::Equality | BinaryOperator::StrictEquality => Some(undef),
                        BinaryOperator::Inequality | BinaryOperator::StrictInequality => {
                            Some(!undef)
                        }
                        _ => None,
                    };
                }
                let abs = is_absent(&b.left) || is_absent(&b.right);
                let nullish_cmp = (is_absent(&b.left) && is_nullish(&b.right))
                    || (is_absent(&b.right) && is_nullish(&b.left));
                if nullish_cmp {
                    // `x == null` / `x === undefined` -> true; `!=` / `!==` -> false.
                    match b.operator {
                        BinaryOperator::Equality | BinaryOperator::StrictEquality => Some(true),
                        BinaryOperator::Inequality | BinaryOperator::StrictInequality => {
                            Some(false)
                        }
                        _ => None,
                    }
                } else if abs {
                    // Relational comparison with `undefined` is always false (NaN).
                    match b.operator {
                        BinaryOperator::GreaterThan
                        | BinaryOperator::GreaterEqualThan
                        | BinaryOperator::LessThan
                        | BinaryOperator::LessEqualThan => Some(false),
                        _ => None,
                    }
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn collect_statement(
        &mut self,
        statement: &'ast Statement<'ast>,
        owner: &'db FunctionFact,
        env: &mut FlowEnv,
    ) {
        match statement {
            Statement::VariableDeclaration(variable) => {
                self.collect_variable_declaration(variable, owner, env);
            }
            Statement::ExpressionStatement(statement) => {
                self.collect_expression(&statement.expression, owner, env);
            }
            Statement::ForOfStatement(statement) => {
                let iterable_targets =
                    self.targets_for_iterable(&statement.right, env, statement.span.start);
                self.collect_for_of_bindings(
                    &statement.left,
                    &iterable_targets,
                    statement.body.span(),
                    owner,
                );
                self.collect_statement(&statement.body, owner, env);
            }
            Statement::BlockStatement(block) => {
                for statement in &block.body {
                    self.collect_statement(statement, owner, env);
                }
            }
            Statement::IfStatement(statement) => {
                self.collect_expression(&statement.test, owner, env);
                match self.decide_branch(&statement.test, env) {
                    Some(true) => self.collect_statement(&statement.consequent, owner, env),
                    Some(false) => {
                        if let Some(alternate) = &statement.alternate {
                            self.collect_statement(alternate, owner, env);
                        }
                    }
                    None => {
                        self.collect_statement(&statement.consequent, owner, env);
                        if let Some(alternate) = &statement.alternate {
                            self.collect_statement(alternate, owner, env);
                        }
                    }
                }
            }
            Statement::ReturnStatement(statement) => {
                if let Some(argument) = &statement.argument {
                    self.collect_expression(argument, owner, env);
                }
            }
            Statement::ForInStatement(statement) => {
                // `for (const k in obj) … obj[k]() …`: mark `k` as an iteration
                // key so a computed access keyed on it ranges over all properties.
                if let ForStatementLeft::VariableDeclaration(variable) = &statement.left {
                    for declarator in &variable.declarations {
                        if let Some(name) = binding_identifier_name(&declarator.id) {
                            env.forin_keys.insert(name);
                        }
                    }
                }
                self.collect_statement(&statement.body, owner, env);
            }
            _ => {}
        }
    }

    fn collect_variable_declaration(
        &mut self,
        variable: &'ast VariableDeclaration<'ast>,
        owner: &'db FunctionFact,
        env: &mut FlowEnv,
    ) {
        for declarator in &variable.declarations {
            // `var app = exports = module.exports = {}` aliases the export object;
            // later `app.X = fn` / `app[k] = fn` writes build the exports. Seed an
            // empty object so those member writes land (member assignment requires
            // the receiver to already be in `env.objects`).
            if let Some(name) = binding_identifier_name(&declarator.id)
                && let Some(init) = &declarator.init
                && expression_aliases_exports(init)
            {
                self.export_aliases.insert(name.clone());
                env.objects.entry(name).or_default();
            }
            // `var mixin = require('merge-descriptors')`: a property-copy helper.
            if let Some(name) = binding_identifier_name(&declarator.id)
                && let Some(Expression::CallExpression(call)) = &declarator.init
                && self.is_merge_descriptors_require(call)
            {
                env.merge_helpers.insert(name);
            }
            if let Some(name) = binding_identifier_name(&declarator.id)
                && let Some(Expression::ClassExpression(class)) = &declarator.init
            {
                self.classes.insert(name.clone(), self.class_targets(class));
                if let Some(statics) = self.class_static_targets(&name) {
                    env.objects.insert(name, statics);
                }
            }
            // Bind a variable to a class flowed through it (returned from a
            // function, aliased, or a direct class expression) so `new v()` /
            // `v.staticM()` resolve. Seed the static object for static calls.
            if let Some(name) = binding_identifier_name(&declarator.id)
                && let Some(init) = &declarator.init
                && let Some(key) = self.class_key_from_expression(init, env)
            {
                if let Some(statics) = self.class_static_targets(&key) {
                    env.objects.insert(name.clone(), statics);
                }
                env.class_bindings.insert(name, key);
            }
            if let Some(name) = binding_identifier_name(&declarator.id)
                && let Some(targets) = self.targets_for_declarator_init(declarator, env)
            {
                env.bindings.insert(name, targets);
            }
            if let Some(name) = binding_identifier_name(&declarator.id)
                && let Some(init) = &declarator.init
                && let Some(value) = self.constant_string_from_expression(init, env)
            {
                env.string_bindings.insert(name, value);
            }
            if let Some(name) = binding_identifier_name(&declarator.id)
                && let Some(init) = &declarator.init
                && let Some(value) = self.constant_bool_from_expression(init, env)
            {
                env.bool_bindings.insert(name, value);
            }
            if let Some(name) = binding_identifier_name(&declarator.id)
                && let Some(Expression::ArrayExpression(array)) = &declarator.init
                && let Some(values) = self.string_array_from_literal(array, env)
            {
                env.string_arrays.insert(name, values);
            }
            if let Some(name) = binding_identifier_name(&declarator.id)
                && let Some(init) = &declarator.init
                && let Expression::CallExpression(call) = init
                && let Some(bound_targets) = self.bound_function_targets_from_bind_call(call, env)
            {
                env.bindings.insert(
                    name.clone(),
                    CollectionTargets {
                        keys: Vec::new(),
                        values: bound_targets.iter().map(|target| target.function).collect(),
                        object_values: Vec::new(),
                        ..Default::default()
                    },
                );
                env.bound_functions.insert(name, bound_targets);
            }
            // `const l = o.foo()` where `foo` returns `() => this.bar()`: bind `l` to
            // the arrow with captured `this = o`, so `l()` walks the arrow body with
            // that `this` (and a never-called `l` emits nothing).
            if let Some(name) = binding_identifier_name(&declarator.id)
                && let Some(Expression::CallExpression(call)) = &declarator.init
            {
                let bound_closures = self.bound_closures_from_call(call, env);
                if !bound_closures.is_empty() {
                    env.bound_functions
                        .entry(name)
                        .or_default()
                        .extend(bound_closures);
                }
            }
            if let Some(name) = binding_identifier_name(&declarator.id)
                && let Some(init) = &declarator.init
            {
                let returned = self.callable_return_targets_from_expression(init, env, 0);
                if !returned.is_empty() {
                    env.bindings.insert(name, returned);
                }
            }
            if let Some(name) = binding_identifier_name(&declarator.id)
                && let Some(init) = &declarator.init
                && let Some(function) = self.function_for_expression(init)
            {
                env.bindings.insert(
                    name,
                    CollectionTargets {
                        keys: Vec::new(),
                        values: vec![function.id],
                        object_values: Vec::new(),
                        ..Default::default()
                    },
                );
            }
            // A guarded callable binding: `var __importDefault = (this &&
            // this.__importDefault) || function (mod) { … }` — the preamble
            // TypeScript and Babel emit for CommonJS default interop. The guard
            // branch is an unknown member load, so only the literal function
            // branch has a declaration; bind it so a call *through the helper
            // name* resolves to the helper the file actually declares.
            if let Some(name) = binding_identifier_name(&declarator.id)
                && let Some(init) = &declarator.init
            {
                let guarded = self.guarded_callable_functions(init);
                if !guarded.is_empty() {
                    let targets = env.bindings.entry(name).or_default();
                    for function in guarded {
                        if !targets.values.contains(&function) {
                            targets.values.push(function);
                        }
                    }
                }
            }
            if let Some(name) = binding_identifier_name(&declarator.id)
                && let Some(init) = &declarator.init
                && let Some(targets) = self.async_function_promise_targets(init, env)
            {
                env.async_functions.insert(name, targets);
            }
            if let Some(name) = binding_identifier_name(&declarator.id)
                && let Some(init) = &declarator.init
                && let Some(targets) = self.async_generator_yield_targets(init, env)
            {
                env.async_generators.insert(name, targets);
            }
            if let Some(name) = binding_identifier_name(&declarator.id)
                && let Some(init) = &declarator.init
                && let Some(sequence) = self.async_generator_sequence_targets(init, env)
            {
                env.generator_sequences.insert(name, sequence);
            }
            if let Some(name) = binding_identifier_name(&declarator.id)
                && let Some(init) = &declarator.init
                && let Some(targets) = self.generator_iterator_targets(init, env)
            {
                env.async_iterators.insert(name, targets);
            }
            if let Some(name) = binding_identifier_name(&declarator.id)
                && let Some(init) = &declarator.init
                && let Some(sequence) = self.generator_iterator_sequence(init, env)
            {
                env.iterator_sequences.insert(name, sequence);
            }
            if let Some(init) = &declarator.init
                && let Some(targets) = self.collection_targets_from_expression(init, env)
            {
                bind_collection_pattern(&declarator.id, &targets, env);
            }
            if let Some(init) = &declarator.init {
                let mut targets = self.object_targets_from_expression(init, env);
                // Object destructuring from a call to a same-file function that
                // returns an object (`const {a, b} = make()`) needs the returned
                // object's shape, which `object_targets_from_expression` (read-only)
                // cannot build; fall back to walking the callee body. Restricted to
                // object patterns so plain `const x = factory()` does not re-walk
                // the callee (it would re-emit its edges, occasionally over-resolved).
                if targets.is_none() && matches!(&declarator.id, BindingPattern::ObjectPattern(_)) {
                    targets = self.object_targets_from_local_function_call(init, env);
                }
                if let Some(targets) = targets {
                    self.collect_object_pattern_binding(&declarator.id, &targets, env);
                }
            }
            if let Some(name) = binding_identifier_name(&declarator.id)
                && let Some(init) = &declarator.init
                && let Some(targets) = self.promise_targets_from_expression(init, env)
            {
                env.promises.insert(name, targets);
            }
            if let Some(name) = binding_identifier_name(&declarator.id)
                && let Some(Expression::ObjectExpression(object)) = &declarator.init
            {
                env.objects
                    .insert(name, self.object_literal_targets(object, Some(env)));
            }
            if let Some(init) = &declarator.init {
                self.collect_expression(init, owner, env);
            }
        }
    }

    fn collect_expression(
        &mut self,
        expression: &'ast Expression<'ast>,
        owner: &'db FunctionFact,
        env: &mut FlowEnv,
    ) {
        match expression {
            Expression::CallExpression(call) => {
                self.collect_call_expression(call, owner, env);
                self.collect_call_callee_expression(&call.callee, owner, env);
                for argument in &call.arguments {
                    if let Some(expression) = argument_expression(argument) {
                        self.collect_expression(expression, owner, env);
                    }
                }
            }
            Expression::NewExpression(new) => {
                // `new v()` where `v` is a variable bound to a class flowed through
                // it: emit the constructor edge (direct `new ClassName()` is already
                // resolved by the direct/MIR path).
                if let Some(name) = callee_identifier(&new.callee)
                    && let Some(key) = env.class_bindings.get(name)
                    && let Some(constructor) =
                        self.classes.get(key).and_then(|class| class.constructor)
                {
                    self.emit_call_span_targets(owner, new.span, "new", Some(vec![constructor]));
                }
                for argument in &new.arguments {
                    if let Some(expression) = argument_expression(argument) {
                        self.collect_expression(expression, owner, env);
                    }
                }
            }
            Expression::AwaitExpression(expression) => {
                self.collect_expression(&expression.argument, owner, env);
            }
            Expression::TaggedTemplateExpression(tagged) => {
                self.collect_tagged_template_expression(tagged, owner, env);
            }
            Expression::AssignmentExpression(assignment) => {
                self.collect_assignment_value_flow(assignment, env);
                if let Some(collection_name) = collection_assignment_target_name(&assignment.left)
                    && let Some(function) = self.function_for_expression(&assignment.right)
                    && let Some(targets) = env.bindings.get_mut(collection_name)
                {
                    // A constant index write that extends the array by one
                    // (`arr[arr.length] = fn`) keeps a precise numeric cell, so
                    // rest-destructuring and `arr[i]` reads still resolve it. Any
                    // other computed write — a non-constant key (`arr[k] = fn`) or an
                    // out-of-range / overwriting index — has no precise position, so
                    // it joins the dynamic bucket: `for…of` sees it but a specific
                    // index read and `pop` stay precise (see the `dynamic` field). A
                    // static-member write (`coll.prop = fn`) keeps its value lane.
                    match &assignment.left {
                        AssignmentTarget::ComputedMemberExpression(member) => {
                            match numeric_index(&member.expression) {
                                Some(index) if index == targets.values.len() => {
                                    targets.values.push(function.id);
                                }
                                _ => targets.push_dynamic(vec![function.id]),
                            }
                        }
                        _ => targets.values.push(function.id),
                    }
                }
                self.collect_expression(&assignment.right, owner, env);
            }
            Expression::ArrayExpression(array) => {
                for element in &array.elements {
                    if let Some(expression) = array_element_expression(element) {
                        self.collect_expression(expression, owner, env);
                    }
                }
            }
            Expression::ObjectExpression(object) => {
                for property in &object.properties {
                    match property {
                        ObjectPropertyKind::ObjectProperty(property) => {
                            self.collect_expression(&property.value, owner, env);
                        }
                        ObjectPropertyKind::SpreadProperty(spread) => {
                            self.collect_expression(&spread.argument, owner, env);
                        }
                    }
                }
            }
            Expression::ParenthesizedExpression(expression) => {
                self.collect_expression(&expression.expression, owner, env);
            }
            Expression::SequenceExpression(sequence) => {
                for expression in &sequence.expressions {
                    self.collect_expression(expression, owner, env);
                }
            }
            // Calls buried inside operator/template/member positions still need to
            // resolve — e.g. mocha's `assert.ok(typeof mylib.plus(4,2) === 'number')`
            // hides the call under `typeof … === …`. Descend into every
            // sub-expression so the nested call is reached.
            Expression::BinaryExpression(binary) => {
                self.collect_expression(&binary.left, owner, env);
                self.collect_expression(&binary.right, owner, env);
            }
            Expression::LogicalExpression(logical) => {
                self.collect_expression(&logical.left, owner, env);
                self.collect_expression(&logical.right, owner, env);
            }
            Expression::UnaryExpression(unary) => {
                self.collect_expression(&unary.argument, owner, env);
            }
            Expression::ConditionalExpression(conditional) => {
                self.collect_expression(&conditional.test, owner, env);
                self.collect_expression(&conditional.consequent, owner, env);
                self.collect_expression(&conditional.alternate, owner, env);
            }
            Expression::TemplateLiteral(template) => {
                for expression in &template.expressions {
                    self.collect_expression(expression, owner, env);
                }
            }
            Expression::StaticMemberExpression(member) => {
                self.collect_expression(&member.object, owner, env);
            }
            Expression::ComputedMemberExpression(member) => {
                self.collect_expression(&member.object, owner, env);
                self.collect_expression(&member.expression, owner, env);
            }
            Expression::YieldExpression(expression) => {
                if let Some(argument) = &expression.argument {
                    self.collect_expression(argument, owner, env);
                }
            }
            _ => {}
        }
    }

    fn collect_assignment_value_flow(
        &mut self,
        assignment: &'ast oxc_ast::ast::AssignmentExpression<'ast>,
        env: &mut FlowEnv,
    ) {
        self.collect_commonjs_export_assignment(assignment, env);
        if let Some((class_name, super_name)) =
            prototype_assignment_super_name(&assignment.left, &assignment.right)
        {
            self.classes
                .entry(class_name.to_string())
                .or_default()
                .super_name = Some(super_name.to_string());
        }

        // `C.prototype = { foo() {} }` (or a variable holding such an object):
        // the object's members become C's instance methods, so `new C().foo()`
        // resolves. (`C.prototype = new S()` is handled above as a super link.)
        if let Some(class_name) = prototype_object_assignment_name(&assignment.left)
            && !matches!(&assignment.right, Expression::NewExpression(_))
            && let Some(object) = self.object_targets_from_expression(&assignment.right, env)
        {
            self.classes
                .entry(class_name.to_string())
                .or_default()
                .instance_object
                .merge(object);
            return;
        }

        // `obj.__proto__ = proto` links `obj`'s prototype, so `obj` inherits the
        // prototype's members (same effect as `Object.setPrototypeOf`).
        if let Some(object_name) = proto_assignment_object_name(&assignment.left)
            && let Some(proto) = self.object_targets_from_expression(&assignment.right, env)
            && let Some(target) = env.objects.get_mut(object_name)
        {
            target.merge(proto.clone());
            env.object_prototypes.insert(object_name.to_string(), proto);
            return;
        }

        let this_properties = self.this_assignment_property_names(&assignment.left, env);
        if !this_properties.is_empty() {
            let targets = self
                .callable_targets_from_expression(&assignment.right, env)
                .all_targets();
            let nested_object = self.object_targets_from_expression(&assignment.right, env);
            for property in this_properties {
                for target in &targets {
                    env.this_object
                        .add_property_target(property.clone(), *target);
                }
                if let Some(nested_object) = nested_object.clone() {
                    env.this_object.add_object_property(property, nested_object);
                }
            }
            return;
        }

        if let Some((class_name, property)) = prototype_member_assignment_name(&assignment.left) {
            let property = property.to_string();
            let targets = self
                .callable_targets_from_expression(&assignment.right, env)
                .all_targets();
            if !targets.is_empty() {
                // A function-constructor (`function IPv4(){}`) is not yet in the
                // class table; the first `IPv4.prototype.m = fn` creates it, so
                // classic prototype OOP registers its instance methods.
                let class = self.classes.entry(class_name.to_string()).or_default();
                for target in targets {
                    class
                        .instance_object
                        .add_property_target(property.clone(), target);
                }
            }
            return;
        }

        let assignments = self.assignment_member_names(&assignment.left, env);
        if assignments.is_empty() {
            return;
        }
        let callable_targets = self
            .callable_targets_from_expression(&assignment.right, env)
            .all_targets();
        let nested_object = self.object_targets_from_expression(&assignment.right, env);
        let nested_collection = self.collection_targets_from_expression(&assignment.right, env);

        for (object_name, property) in assignments {
            if self.classes.contains_key(object_name)
                && !env.objects.contains_key(object_name)
                && let Some(statics) = self.class_static_targets(object_name)
            {
                env.objects.insert(object_name.to_string(), statics);
            }
            // A function declaration is also an object: `function f(){}; f.g = fn`
            // attaches `g` to f, so `f.g()` resolves and `this` inside `f.g` is the
            // f-object (Jelly tracks `this` to the function's allocation site).
            if !env.objects.contains_key(object_name)
                && self.function_declarations.contains_key(object_name)
            {
                env.objects
                    .insert(object_name.to_string(), ObjectTargets::default());
            }

            let setter_targets = env
                .objects
                .get(object_name)
                .and_then(|object| object.setter_properties.get(&property))
                .cloned()
                .unwrap_or_default();
            let handled_by_setter = !setter_targets.is_empty();
            if handled_by_setter {
                self.collect_setter_assignment_side_effects(
                    object_name,
                    &setter_targets,
                    &assignment.right,
                    env,
                );
            } else if let Some(object) = env.objects.get_mut(object_name) {
                add_assignment_to_object_property(
                    object,
                    property.clone(),
                    &callable_targets,
                    nested_object.clone(),
                    nested_collection.clone(),
                );
            }

            if self.classes.contains_key(object_name) {
                if handled_by_setter {
                    if let Some(object) = env.objects.get(object_name).cloned()
                        && let Some(class) = self.classes.get_mut(object_name)
                    {
                        class.static_object = object;
                    }
                } else if let Some(class) = self.classes.get_mut(object_name) {
                    add_assignment_to_object_property(
                        &mut class.static_object,
                        property,
                        &callable_targets,
                        nested_object.clone(),
                        nested_collection.clone(),
                    );
                }
                if let Some(statics) = self.class_static_targets(object_name) {
                    env.objects.insert(object_name.to_string(), statics);
                }
            }
        }
    }

    /// Capture CommonJS export writes into `self.exports`: `module.exports = X`,
    /// `module.exports.foo = X`, and `exports.foo = X`. This is what later
    /// rounds of the summary fixpoint (and requiring modules) read back.
    fn collect_commonjs_export_assignment(
        &mut self,
        assignment: &'ast oxc_ast::ast::AssignmentExpression<'ast>,
        env: &FlowEnv,
    ) {
        let Some(target) = export_assignment_target(&assignment.left) else {
            return;
        };
        let mut callables = self.callable_targets_from_expression(&assignment.right, env);
        // An identifier RHS may name a top-level function declaration (e.g.
        // `function foo() {}` ... `module.exports = foo`), which lives in
        // `function_declarations` rather than `env.bindings`.
        if let Expression::Identifier(identifier) = &assignment.right
            && let Some(flow) = self.function_declarations.get(identifier.name.as_str())
            && !callables.values.contains(&flow.function.id)
        {
            callables.values.push(flow.function.id);
        }
        let object = self.object_targets_from_expression(&assignment.right, env);
        let collection = self.collection_targets_from_expression(&assignment.right, env);
        // `module.exports = require('./x')` re-exports another module: fold its
        // summary in directly so callers see the re-exported shape.
        let reexport = match &assignment.right {
            Expression::CallExpression(call) => self.require_summary(call).cloned(),
            _ => None,
        };
        match target {
            ExportAssignmentTarget::WholeExports => {
                self.exports.callables.extend(callables);
                if let Some(object) = object {
                    self.exports.object.merge(object);
                }
                if let Some(reexport) = reexport {
                    self.exports.callables.extend(reexport.callables);
                    self.exports.object.merge(reexport.object);
                }
            }
            ExportAssignmentTarget::Property(name) => {
                for target in callables.all_targets() {
                    self.exports
                        .object
                        .add_property_target(name.clone(), target);
                }
                if let Some(object) = object {
                    self.exports
                        .object
                        .add_object_property(name.clone(), object);
                }
                if let Some(collection) = collection {
                    self.exports
                        .object
                        .add_collection_property(name, collection);
                }
            }
        }
    }

    /// Resolve a `require`/`import`/`export ... from` specifier string to the
    /// converged export summary of the target module, if any.
    fn summary_for_specifier(&self, specifier: &str) -> Option<&'env ModuleExportSummary> {
        let target = self
            .resolution_map
            .get(&(self.file.id, specifier.to_string()))?;
        self.module_summaries.get(target)
    }

    /// Bind ESM imports for this module into `env` before the body is walked.
    fn collect_esm_imports(&self, program: &'ast Program<'ast>, env: &mut FlowEnv) {
        for statement in &program.body {
            let Statement::ImportDeclaration(import) = statement else {
                continue;
            };
            let Some(summary) = self.summary_for_specifier(import.source.value.as_str()) else {
                continue;
            };
            let summary = summary.clone();
            let Some(specifiers) = &import.specifiers else {
                continue;
            };
            for specifier in specifiers {
                match specifier {
                    ImportDeclarationSpecifier::ImportDefaultSpecifier(default) => {
                        self.bind_imported_value(
                            &summary,
                            "default",
                            default.local.name.as_str(),
                            true,
                            env,
                        );
                    }
                    ImportDeclarationSpecifier::ImportNamespaceSpecifier(namespace) => {
                        if !summary.object.is_empty() {
                            env.objects
                                .insert(namespace.local.name.to_string(), summary.object.clone());
                        }
                        if !summary.callables.is_empty() {
                            env.bindings.insert(
                                namespace.local.name.to_string(),
                                summary.callables.clone(),
                            );
                        }
                    }
                    ImportDeclarationSpecifier::ImportSpecifier(named) => {
                        let imported = module_export_name_string(&named.imported);
                        self.bind_imported_value(
                            &summary,
                            &imported,
                            named.local.name.as_str(),
                            imported == "default",
                            env,
                        );
                    }
                }
            }
        }
    }

    /// Bind a single imported name `local` to the value of `key` in `summary`.
    fn bind_imported_value(
        &self,
        summary: &ModuleExportSummary,
        key: &str,
        local: &str,
        is_default: bool,
        env: &mut FlowEnv,
    ) {
        let mut callables = summary
            .object
            .properties
            .get(key)
            .cloned()
            .unwrap_or_default();
        // CommonJS default-interop: `import x from './cjs'` where the module did
        // `module.exports = fn` exposes the function as the default.
        if is_default {
            callables.extend(summary.callables.values.iter().copied());
        }
        if !callables.is_empty() {
            env.bindings.insert(
                local.to_string(),
                CollectionTargets {
                    keys: Vec::new(),
                    values: callables,
                    object_values: Vec::new(),
                    ..Default::default()
                },
            );
        }
        if let Some(object) = summary.object.object_properties.get(key) {
            env.objects.insert(local.to_string(), (**object).clone());
        } else if is_default && !summary.object.is_empty() && summary.callables.is_empty() {
            // CommonJS default-interop for object exports: the whole exports
            // object is the default value.
            env.objects
                .insert(local.to_string(), summary.object.clone());
        }
        if let Some(collection) = summary.object.collection_properties.get(key) {
            env.bindings.insert(local.to_string(), collection.clone());
        }
    }

    /// Capture ESM exports into `self.exports`.
    fn collect_esm_export(&mut self, statement: &'ast Statement<'ast>, env: &mut FlowEnv) {
        match statement {
            Statement::ExportNamedDeclaration(export) => {
                if let Some(source) = &export.source {
                    // Re-export: `export { a, b as c } from './m'`.
                    let Some(summary) = self.summary_for_specifier(source.value.as_str()) else {
                        return;
                    };
                    let summary = summary.clone();
                    for specifier in &export.specifiers {
                        let local = module_export_name_string(&specifier.local);
                        let exported = module_export_name_string(&specifier.exported);
                        self.reexport_property(&summary, &local, &exported);
                    }
                } else if let Some(declaration) = &export.declaration {
                    self.capture_exported_declaration(declaration, env);
                } else {
                    // Local: `export { a, b as c }`.
                    for specifier in &export.specifiers {
                        let local = module_export_name_string(&specifier.local);
                        let exported = module_export_name_string(&specifier.exported);
                        self.export_local_name(&local, &exported, env);
                    }
                }
            }
            Statement::ExportDefaultDeclaration(export) => {
                self.capture_default_export(&export.declaration, env);
            }
            Statement::ExportAllDeclaration(export) => {
                if export.exported.is_none()
                    && let Some(summary) = self.summary_for_specifier(export.source.value.as_str())
                {
                    let summary = summary.clone();
                    self.exports.object.merge(summary.object);
                    self.exports.callables.extend(summary.callables);
                }
            }
            _ => {}
        }
    }

    /// Copy property `local` from a re-exported module's summary into this
    /// module's exports under name `exported`.
    fn reexport_property(&mut self, summary: &ModuleExportSummary, local: &str, exported: &str) {
        let mut callables = summary
            .object
            .properties
            .get(local)
            .cloned()
            .unwrap_or_default();
        if local == "default" {
            callables.extend(summary.callables.values.iter().copied());
        }
        for target in callables {
            self.exports
                .object
                .add_property_target(exported.to_string(), target);
        }
        if let Some(object) = summary.object.object_properties.get(local) {
            self.exports
                .object
                .add_object_property(exported.to_string(), (**object).clone());
        }
        if let Some(collection) = summary.object.collection_properties.get(local) {
            self.exports
                .object
                .add_collection_property(exported.to_string(), collection.clone());
        }
    }

    /// Export a locally-bound name (`export { foo }` / `export { foo as bar }`).
    fn export_local_name(&mut self, local: &str, exported: &str, env: &FlowEnv) {
        if let Some(flow) = self.function_declarations.get(local) {
            self.exports
                .object
                .add_property_target(exported.to_string(), flow.function.id);
        }
        if let Some(targets) = env.bindings.get(local) {
            for target in targets.all_targets() {
                self.exports
                    .object
                    .add_property_target(exported.to_string(), target);
            }
        }
        if let Some(object) = env.objects.get(local) {
            self.exports
                .object
                .add_object_property(exported.to_string(), object.clone());
        }
    }

    /// Capture `export const/var/function/class ...` declarations.
    fn capture_exported_declaration(
        &mut self,
        declaration: &'ast Declaration<'ast>,
        env: &mut FlowEnv,
    ) {
        match declaration {
            Declaration::FunctionDeclaration(function) => {
                if let Some(id) = &function.id
                    && let Some(fact) = self.function_for_span(function.span)
                {
                    self.exports
                        .object
                        .add_property_target(id.name.to_string(), fact.id);
                }
            }
            Declaration::VariableDeclaration(variable) => {
                self.collect_variable_declaration(variable, self.module, env);
                for declarator in &variable.declarations {
                    if let Some(name) = binding_identifier_name(&declarator.id) {
                        self.export_local_name(&name, &name, env);
                    }
                }
            }
            _ => {}
        }
    }

    /// Capture `export default ...`.
    fn capture_default_export(
        &mut self,
        declaration: &'ast ExportDefaultDeclarationKind<'ast>,
        env: &FlowEnv,
    ) {
        let mut callables = CollectionTargets::default();
        match declaration {
            ExportDefaultDeclarationKind::FunctionDeclaration(function) => {
                if let Some(fact) = self.function_for_span(function.span) {
                    callables.values.push(fact.id);
                }
            }
            expression => {
                if let Some(expression) = expression.as_expression() {
                    callables = self.callable_targets_from_expression(expression, env);
                    if let Some(object) = self.object_targets_from_expression(expression, env) {
                        self.exports
                            .object
                            .add_object_property("default".to_string(), object);
                    }
                }
            }
        }
        for target in callables.all_targets() {
            self.exports
                .object
                .add_property_target("default".to_string(), target);
            self.exports.callables.values.push(target);
        }
        self.exports.callables.values.sort();
        self.exports.callables.values.dedup();
    }

    fn assignment_member_names(
        &self,
        target: &'ast AssignmentTarget<'ast>,
        env: &FlowEnv,
    ) -> Vec<(&'ast str, String)> {
        match target {
            AssignmentTarget::StaticMemberExpression(member) => {
                let Some(object) = expression_identifier(&member.object) else {
                    return Vec::new();
                };
                vec![(object, member.property.name.to_string())]
            }
            AssignmentTarget::ComputedMemberExpression(member) => {
                let Some(object) = expression_identifier(&member.object) else {
                    return Vec::new();
                };
                self.computed_member_property_names(&member.expression, env)
                    .into_iter()
                    .map(|property| (object, property))
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    fn computed_member_property_names(
        &self,
        expression: &'ast Expression<'ast>,
        env: &FlowEnv,
    ) -> Vec<String> {
        let mut properties = match expression {
            Expression::NumericLiteral(literal) => vec![literal.value.to_string()],
            Expression::ParenthesizedExpression(expression) => {
                self.computed_member_property_names(&expression.expression, env)
            }
            _ => self.constant_strings_from_expression(expression, env),
        };
        sort_dedup_strings(&mut properties);
        properties.truncate(8);
        properties
    }

    fn constant_string_from_expression(
        &self,
        expression: &'ast Expression<'ast>,
        env: &FlowEnv,
    ) -> Option<String> {
        self.constant_strings_from_expression(expression, env)
            .into_iter()
            .next()
    }

    fn constant_strings_from_expression(
        &self,
        expression: &'ast Expression<'ast>,
        env: &FlowEnv,
    ) -> Vec<String> {
        match expression {
            Expression::StringLiteral(literal) => vec![literal.value.to_string()],
            Expression::Identifier(identifier) => env
                .string_bindings
                .get(identifier.name.as_str())
                .cloned()
                .into_iter()
                .collect(),
            Expression::BinaryExpression(binary) if binary.operator == BinaryOperator::Addition => {
                let left = self.constant_strings_from_expression(&binary.left, env);
                let right = self.constant_strings_from_expression(&binary.right, env);
                if left.is_empty() || right.is_empty() {
                    return Vec::new();
                }
                bounded_string_product(&left, &right)
            }
            Expression::ConditionalExpression(conditional) => {
                if let Some(value) = self.constant_bool_from_expression(&conditional.test, env) {
                    if value {
                        self.constant_strings_from_expression(&conditional.consequent, env)
                    } else {
                        self.constant_strings_from_expression(&conditional.alternate, env)
                    }
                } else {
                    let mut values =
                        self.constant_strings_from_expression(&conditional.consequent, env);
                    values
                        .extend(self.constant_strings_from_expression(&conditional.alternate, env));
                    sort_dedup_strings(&mut values);
                    values.truncate(8);
                    values
                }
            }
            Expression::ComputedMemberExpression(member) => {
                let Some(source) = expression_identifier(&member.object)
                    .and_then(|name| env.string_arrays.get(name))
                else {
                    return Vec::new();
                };
                let Some(index) = numeric_index(&member.expression) else {
                    return Vec::new();
                };
                source.get(index).cloned().into_iter().collect()
            }
            Expression::TemplateLiteral(template) if template.expressions.is_empty() => template
                .quasis
                .iter()
                .map(|quasi| {
                    quasi
                        .value
                        .cooked
                        .as_ref()
                        .unwrap_or(&quasi.value.raw)
                        .to_string()
                })
                .collect(),
            Expression::TemplateLiteral(template) => {
                let mut values = vec![String::new()];
                for (index, quasi) in template.quasis.iter().enumerate() {
                    let text = quasi
                        .value
                        .cooked
                        .as_ref()
                        .unwrap_or(&quasi.value.raw)
                        .to_string();
                    for value in &mut values {
                        value.push_str(&text);
                    }
                    if let Some(expression) = template.expressions.get(index) {
                        let parts = self.constant_strings_from_expression(expression, env);
                        if parts.is_empty() {
                            return Vec::new();
                        }
                        values = bounded_string_product(&values, &parts);
                    }
                }
                values
            }
            Expression::ParenthesizedExpression(expression) => {
                self.constant_strings_from_expression(&expression.expression, env)
            }
            _ => Vec::new(),
        }
    }

    fn constant_bool_from_expression(
        &self,
        expression: &'ast Expression<'ast>,
        env: &FlowEnv,
    ) -> Option<bool> {
        match expression {
            Expression::BooleanLiteral(literal) => Some(literal.value),
            Expression::Identifier(identifier) => {
                env.bool_bindings.get(identifier.name.as_str()).copied()
            }
            Expression::BinaryExpression(binary)
                if matches!(
                    binary.operator,
                    BinaryOperator::Equality | BinaryOperator::StrictEquality
                ) =>
            {
                if let (Some(left), Some(right)) = (
                    self.constant_bool_from_expression(&binary.left, env),
                    self.constant_bool_from_expression(&binary.right, env),
                ) {
                    return Some(left == right);
                }
                if let (Some(left), Some(right)) = (
                    self.constant_string_from_expression(&binary.left, env),
                    self.constant_string_from_expression(&binary.right, env),
                ) {
                    return Some(left == right);
                }
                None
            }
            Expression::BinaryExpression(binary)
                if matches!(
                    binary.operator,
                    BinaryOperator::Inequality | BinaryOperator::StrictInequality
                ) =>
            {
                if let (Some(left), Some(right)) = (
                    self.constant_bool_from_expression(&binary.left, env),
                    self.constant_bool_from_expression(&binary.right, env),
                ) {
                    return Some(left != right);
                }
                if let (Some(left), Some(right)) = (
                    self.constant_string_from_expression(&binary.left, env),
                    self.constant_string_from_expression(&binary.right, env),
                ) {
                    return Some(left != right);
                }
                None
            }
            Expression::ParenthesizedExpression(expression) => {
                self.constant_bool_from_expression(&expression.expression, env)
            }
            _ => None,
        }
    }

    fn string_array_from_literal(
        &self,
        array: &'ast oxc_ast::ast::ArrayExpression<'ast>,
        env: &FlowEnv,
    ) -> Option<Vec<String>> {
        let mut values = Vec::new();
        for element in &array.elements {
            let expression = array_element_expression(element)?;
            let value = self.constant_string_from_expression(expression, env)?;
            values.push(value);
        }
        Some(values)
    }

    fn property_key_names(
        &self,
        key: &'ast PropertyKey<'ast>,
        computed: bool,
        env: Option<&FlowEnv>,
    ) -> Vec<String> {
        if !computed {
            return static_property_key(key, false).into_iter().collect();
        }

        let empty;
        let env = if let Some(env) = env {
            env
        } else {
            empty = FlowEnv::default();
            &empty
        };

        let mut names = match key {
            PropertyKey::StringLiteral(literal) => vec![literal.value.to_string()],
            PropertyKey::NumericLiteral(literal) => vec![literal.value.to_string()],
            PropertyKey::Identifier(identifier) => env
                .string_bindings
                .get(identifier.name.as_str())
                .cloned()
                .into_iter()
                .collect(),
            PropertyKey::BinaryExpression(binary)
                if binary.operator == BinaryOperator::Addition =>
            {
                let left = self.constant_strings_from_expression(&binary.left, env);
                let right = self.constant_strings_from_expression(&binary.right, env);
                if left.is_empty() || right.is_empty() {
                    Vec::new()
                } else {
                    bounded_string_product(&left, &right)
                }
            }
            PropertyKey::ConditionalExpression(conditional) => {
                if let Some(value) = self.constant_bool_from_expression(&conditional.test, env) {
                    if value {
                        self.constant_strings_from_expression(&conditional.consequent, env)
                    } else {
                        self.constant_strings_from_expression(&conditional.alternate, env)
                    }
                } else {
                    let mut values =
                        self.constant_strings_from_expression(&conditional.consequent, env);
                    values
                        .extend(self.constant_strings_from_expression(&conditional.alternate, env));
                    values
                }
            }
            PropertyKey::ComputedMemberExpression(member) => {
                let Some(source) = expression_identifier(&member.object)
                    .and_then(|name| env.string_arrays.get(name))
                else {
                    return Vec::new();
                };
                let Some(index) = numeric_index(&member.expression) else {
                    return Vec::new();
                };
                source.get(index).cloned().into_iter().collect()
            }
            PropertyKey::TemplateLiteral(template) if template.expressions.is_empty() => template
                .quasis
                .iter()
                .map(|quasi| {
                    quasi
                        .value
                        .cooked
                        .as_ref()
                        .unwrap_or(&quasi.value.raw)
                        .to_string()
                })
                .collect(),
            PropertyKey::ParenthesizedExpression(expression) => {
                self.constant_strings_from_expression(&expression.expression, env)
            }
            _ => Vec::new(),
        };
        sort_dedup_strings(&mut names);
        names.truncate(8);
        names
    }

    fn collect_call_callee_expression(
        &mut self,
        callee: &'ast Expression<'ast>,
        owner: &'db FunctionFact,
        env: &mut FlowEnv,
    ) {
        match callee {
            Expression::CallExpression(call) => {
                self.collect_call_expression(call, owner, env);
                self.collect_call_callee_expression(&call.callee, owner, env);
                for argument in &call.arguments {
                    if let Some(expression) = argument_expression(argument) {
                        self.collect_expression(expression, owner, env);
                    }
                }
            }
            Expression::StaticMemberExpression(member) => {
                self.collect_expression(&member.object, owner, env);
            }
            Expression::ComputedMemberExpression(member) => {
                self.collect_expression(&member.object, owner, env);
                self.collect_expression(&member.expression, owner, env);
            }
            Expression::ParenthesizedExpression(expression) => {
                self.collect_call_callee_expression(&expression.expression, owner, env);
            }
            _ => {}
        }
    }

    fn collect_call_expression(
        &mut self,
        call: &'ast CallExpression<'ast>,
        owner: &'db FunctionFact,
        env: &mut FlowEnv,
    ) {
        self.collect_inline_callee_value_flows(call, env);
        self.collect_native_object_call(call, env);
        self.collect_foreach_computed_write(call, env);
        self.collect_merge_helper_call(call, env);
        self.collect_test_framework_callbacks(call, env);

        if let Some((source, method)) = self.promise_method_source(call, env) {
            self.collect_promise_handler_flows(call, method, &source, env);
        }

        if let Some(targets) = self.collect_function_prototype_method_call(call, owner, env)
            && !targets.is_empty()
        {
            self.emit_call_span_targets(
                owner,
                call.span,
                "call-apply-bind",
                Some(targets.all_targets()),
            );
        }

        if call_expression_callee(&call.callee).is_some() {
            let returned = self.callable_return_targets_from_expression(&call.callee, env, 0);
            self.emit_call_span_targets(
                owner,
                call.span,
                "call-result",
                Some(returned.all_targets()),
            );
            // `o.foo()()`: the result of `o.foo()` is a `this`-capturing arrow; walk
            // its body with the captured `this = o` now that it is actually invoked.
            if let Expression::CallExpression(inner) = &call.callee {
                for bound in self.bound_closures_from_call(inner, env) {
                    self.collect_bound_function_invocation(&bound, call, env);
                }
            }
        }

        if matches!(&call.callee, Expression::ThisExpression(_)) {
            self.emit_call_span_targets(
                owner,
                call.span,
                "this",
                Some(env.this_callables.all_targets()),
            );
        }

        if callee_text(&call.callee).as_deref() == Some("Array.from")
            && let Some(collection_targets) = call
                .arguments
                .first()
                .and_then(argument_expression)
                .and_then(|argument| self.collection_targets_from_expression(argument, env))
            && let Some(callback_expression) = call.arguments.get(1).and_then(argument_expression)
            && let Some(callback) = self.function_for_expression(callback_expression)
        {
            self.collect_callback_parameter_flows(
                callback,
                "from",
                &collection_targets,
                callback_expression,
            );
            if let Some(this_object) = call
                .arguments
                .get(2)
                .and_then(argument_expression)
                .and_then(expression_identifier)
                .and_then(|name| env.objects.get(name))
            {
                self.collect_this_object_member_flows(callback, this_object);
            }
        }

        if let Some((collection_name, method)) = static_member_call(&call.callee) {
            // `arr.push(v)` / `arr.unshift(v)` append at a non-specific index, so the
            // value joins the `%ARRAY_UNKNOWN` bucket — `pop`/`for…of` see it, but a
            // specific-index read (`arr[1]`) stays precise. `Set.add(v)`, by contrast,
            // is the set's element store read back by `.values()`/`for…of`, so it
            // keeps flowing into `values`.
            if matches!(method, "push" | "unshift") {
                let additions = call
                    .arguments
                    .iter()
                    .filter_map(argument_expression)
                    .filter_map(|expression| self.targets_from_argument_expression(expression, env))
                    .collect::<Vec<_>>();
                if let Some(targets) = env.bindings.get_mut(collection_name) {
                    for argument_targets in additions {
                        targets.push_unknown(argument_targets.all_targets());
                        targets
                            .unknown_objects
                            .extend(argument_targets.object_values);
                    }
                }
            }
            if method == "add" {
                let additions = call
                    .arguments
                    .iter()
                    .filter_map(argument_expression)
                    .filter_map(|expression| self.targets_from_argument_expression(expression, env))
                    .collect::<Vec<_>>();
                if let Some(targets) = env.bindings.get_mut(collection_name) {
                    for argument_targets in additions {
                        targets.values.extend(argument_targets.all_targets());
                        targets.object_values.extend(argument_targets.object_values);
                        targets.values.sort();
                        targets.values.dedup();
                    }
                }
            }
            if method == "set"
                && let Some(targets) = env.bindings.get_mut(collection_name)
            {
                if let Some(target) = call
                    .arguments
                    .first()
                    .and_then(argument_expression)
                    .and_then(|expression| self.function_for_expression(expression))
                {
                    targets.keys.push(target.id);
                }
                if let Some(target) = call
                    .arguments
                    .get(1)
                    .and_then(argument_expression)
                    .and_then(|expression| self.function_for_expression(expression))
                {
                    targets.values.push(target.id);
                }
            }
            if let Some(collection_targets) = env.bindings.get(collection_name).cloned()
                && let Some(argument_index) = callback_argument_index(method)
                && let Some(callback) = call
                    .arguments
                    .get(argument_index)
                    .and_then(argument_expression)
                    .and_then(|expression| self.function_for_expression(expression))
            {
                self.collect_callback_parameter_flows(
                    callback,
                    method,
                    &collection_targets,
                    call.arguments
                        .get(argument_index)
                        .and_then(argument_expression)
                        .expect("callback expression already resolved"),
                );
            }
        }

        if let Some((object_name, property)) = static_member_call(&call.callee) {
            let targets = env
                .objects
                .get(object_name)
                .and_then(|object| object.resolve_call_property(property));
            if let Some(targets) = targets {
                self.emit_member_targets(owner, property, call.span, Some(targets.clone()));
                self.collect_receiver_side_effects(object_name, &targets, call, env);
            }

            let getter = env.objects.get(object_name).and_then(|object| {
                object
                    .getter_properties
                    .get(property)
                    .cloned()
                    .map(|targets| (targets, object.clone()))
            });
            if let Some((getters, receiver)) = getter {
                let (returned, receiver) = self.collect_getter_return_targets(getters, receiver, 1);
                env.objects
                    .insert(object_name.to_string(), receiver.clone());
                let targets = returned.all_targets();
                if !targets.is_empty() {
                    self.emit_call_span_targets(owner, call.span, property, Some(targets.clone()));
                    self.emit_member_targets(owner, property, call.span, Some(targets.clone()));
                    self.collect_returned_callable_invocation(
                        Some(object_name),
                        &targets,
                        call,
                        receiver,
                        env,
                    );
                }
            }
        }
        if let Expression::StaticMemberExpression(member) = &call.callee
            && let Some(object) = self.object_targets_from_expression(&member.object, env)
            && let Some(targets) = object.resolve_call_property(member.property.name.as_str())
        {
            self.emit_call_span_targets(
                owner,
                call.span,
                member.property.name.as_str(),
                Some(targets.clone()),
            );
            self.emit_member_targets(
                owner,
                member.property.name.as_str(),
                call.span,
                Some(targets.clone()),
            );
            // Walk the resolved method's body with `this` = the resolved receiver
            // when the receiver is a compound expression (`(o1 || o2).f()`). The
            // plain-identifier receiver path is already covered by
            // `collect_receiver_side_effects`, so restrict to non-identifier
            // receivers to avoid double-walking. (receiver-callee-mixup: `this.g()`
            // inside `f` dispatches to the union of o1/o2's `g`.)
            if expression_identifier(&member.object).is_none() && !object.is_empty() {
                for target in &targets {
                    if let Some(flow) = self.function_flows_by_id.get(target).cloned() {
                        let mut callee_env = env.clone();
                        callee_env.this_object = object.clone();
                        self.collect_function_flow_invocation(&flow, &mut callee_env);
                    }
                }
            }
        }

        // `super.m()` / `super.s()`: resolve against the super-class member
        // object bound while walking this class method body.
        if let Expression::StaticMemberExpression(member) = &call.callee
            && matches!(&member.object, Expression::Super(_))
            && let Some(super_object) = &self.current_super
            && let Some(targets) = super_object
                .properties
                .get(member.property.name.as_str())
                .cloned()
        {
            self.emit_call_span_targets(
                owner,
                call.span,
                member.property.name.as_str(),
                Some(targets.clone()),
            );
            self.emit_member_targets(
                owner,
                member.property.name.as_str(),
                call.span,
                Some(targets.clone()),
            );
            // Walk the resolved super method's body with `this` = the current
            // receiver, so a `this.x()` inside it dispatches against the calling
            // object. (super3: `q2.m2(){ super.m1() }` → `q1.m1(){ this.m3() }`,
            // where `this` is `q2`, resolving `this.m3()` → `q2.m3`.)
            let this_object = env.this_object.clone();
            if !this_object.is_empty() {
                for target in &targets {
                    if let Some(flow) = self.function_flows_by_id.get(target).cloned() {
                        let mut callee_env = env.clone();
                        callee_env.this_object = this_object.clone();
                        self.collect_function_flow_invocation(&flow, &mut callee_env);
                    }
                }
            }
        }

        // Private member calls: `this.#foo()`, `Class.#baz()`. The callee is a
        // PrivateFieldExpression; private members are keyed by `#name`.
        if let Expression::PrivateFieldExpression(member) = &call.callee {
            let property = format!("#{}", member.field.name);
            if let Some(object) = self.object_targets_from_expression(&member.object, env)
                && let Some(targets) = object.properties.get(&property).cloned()
            {
                self.emit_call_span_targets(owner, call.span, &property, Some(targets.clone()));
                self.emit_member_targets(owner, &property, call.span, Some(targets));
            }
        }

        if let Expression::Identifier(identifier) = &call.callee {
            if let Some(flow) = self
                .function_declarations
                .get(identifier.name.as_str())
                .cloned()
            {
                self.collect_function_parameter_flows(&flow, call, env);
            }
            if let Some(bound_targets) = env.bound_functions.get(identifier.name.as_str()).cloned()
            {
                for bound in bound_targets {
                    self.collect_bound_function_invocation(&bound, call, env);
                }
            }
            self.emit_binding_targets(
                owner,
                identifier.name.as_str(),
                call.span,
                env.bindings
                    .get(identifier.name.as_str())
                    .map(CollectionTargets::all_targets),
            );
        }
        if let Expression::ComputedMemberExpression(member) = &call.callee {
            let object_name = expression_identifier(&member.object);
            if let Some(object) = self.object_targets_from_expression(&member.object, env) {
                // A `for-in` key ranges over every own property, so `obj[k]()`
                // dispatches to the union of the object's members.
                let property_names = if expression_identifier(&member.expression)
                    .is_some_and(|name| env.forin_keys.contains(name))
                {
                    object
                        .properties
                        .keys()
                        .chain(object.getter_properties.keys())
                        .cloned()
                        .collect::<Vec<_>>()
                } else {
                    self.computed_member_property_names(&member.expression, env)
                };
                for property in property_names {
                    if let Some(targets) = object.properties.get(&property).cloned() {
                        self.emit_call_span_targets(
                            owner,
                            call.span,
                            &property,
                            Some(targets.clone()),
                        );
                        self.emit_member_targets(
                            owner,
                            &property,
                            call.span,
                            Some(targets.clone()),
                        );
                        if let Some(object_name) = object_name {
                            self.collect_receiver_side_effects(object_name, &targets, call, env);
                        }
                    }
                    if let Some(getters) = object.getter_properties.get(&property).cloned() {
                        let receiver = object_name
                            .and_then(|name| env.objects.get(name).cloned())
                            .unwrap_or_else(|| object.clone());
                        let (returned, receiver) =
                            self.collect_getter_return_targets(getters, receiver, 1);
                        if let Some(object_name) = object_name {
                            env.objects
                                .insert(object_name.to_string(), receiver.clone());
                        }
                        let targets = returned.all_targets();
                        if !targets.is_empty() {
                            self.emit_call_span_targets(
                                owner,
                                call.span,
                                &property,
                                Some(targets.clone()),
                            );
                            self.emit_member_targets(
                                owner,
                                &property,
                                call.span,
                                Some(targets.clone()),
                            );
                            self.collect_returned_callable_invocation(
                                object_name,
                                &targets,
                                call,
                                receiver,
                                env,
                            );
                        }
                    }
                }
            }

            if let Some(source) = self.collection_targets_from_expression(&member.object, env)
                && let Some(index) = numeric_index(&member.expression)
            {
                let binding = callee_text(&call.callee).unwrap_or_else(|| "indexed".to_string());
                self.emit_call_span_targets(
                    owner,
                    call.span,
                    &binding,
                    Some(source.value_at(index).all_targets()),
                );
            }
        }
    }

    /// `coll.forEach(function(k){ obj[k] = fn })` (or arrow) — the verb-attach
    /// idiom Express uses: `methods.forEach(m => { app[m] = handler })`. The
    /// computed key `k` is the callback's element parameter, so the write
    /// attaches `fn` to *every* property of `obj`. Record it on `obj`'s wildcard
    /// lane (the write-side dual of the iter-51 for-in computed read), so a later
    /// `obj.get()` / `obj.post()` resolves to the one handler. Gated to a target
    /// keyed exactly on the callback's first parameter to avoid generalizing.
    fn collect_foreach_computed_write(&self, call: &'ast CallExpression<'ast>, env: &mut FlowEnv) {
        // Method name only — the receiver may be any expression (an array
        // literal `["get","post"]`, a required `methods` binding, …), so match the
        // callee shape directly rather than via `static_member_call` (which
        // requires an identifier receiver).
        let Expression::StaticMemberExpression(callee) = &call.callee else {
            return;
        };
        if !matches!(callee.property.name.as_str(), "forEach" | "map") {
            return;
        }
        let Some(callback) = call.arguments.first().and_then(argument_expression) else {
            return;
        };
        let (first_param, statements) = match callback {
            Expression::FunctionExpression(function) => {
                let Some(body) = function.body.as_deref() else {
                    return;
                };
                (function.params.items.first(), &body.statements)
            }
            Expression::ArrowFunctionExpression(function) => {
                (function.params.items.first(), &function.body.statements)
            }
            _ => return,
        };
        let Some(key_param) = first_param.and_then(|param| binding_identifier_name(&param.pattern))
        else {
            return;
        };
        for statement in statements {
            let Statement::ExpressionStatement(statement) = statement else {
                continue;
            };
            let Expression::AssignmentExpression(assignment) = &statement.expression else {
                continue;
            };
            let AssignmentTarget::ComputedMemberExpression(member) = &assignment.left else {
                continue;
            };
            let Some(object_name) = expression_identifier(&member.object) else {
                continue;
            };
            // The computed key must be exactly the callback's element parameter.
            if expression_identifier(&member.expression) != Some(key_param.as_str()) {
                continue;
            }
            let targets = self
                .callable_targets_from_expression(&assignment.right, env)
                .all_targets();
            if targets.is_empty() {
                continue;
            }
            let object = env.objects.entry(object_name.to_string()).or_default();
            for target in targets {
                object.add_wildcard_target(target);
            }
        }
    }

    /// Is this `require('merge-descriptors')` (possibly through a CommonJS interop
    /// wrapper)? Its result is the property-copy helper Express binds as `mixin`.
    fn is_merge_descriptors_require(&self, call: &'ast CallExpression<'ast>) -> bool {
        if self.require_specifier(call) == Some("merge-descriptors") {
            return true;
        }
        if let Some(Expression::CallExpression(inner)) = self.commonjs_interop_argument(call) {
            return self.require_specifier(inner) == Some("merge-descriptors");
        }
        false
    }

    /// `mixin(dst, src, …)` where `mixin` is a `merge-descriptors` helper: copy
    /// every own property of each source onto `dst` in place (creating `dst`'s
    /// object entry if the variable holds a function, as `createApplication`'s
    /// `var app = function(){…}` does). This is the same merge as the named-target
    /// `Object.assign`, behind a required helper.
    fn collect_merge_helper_call(&mut self, call: &'ast CallExpression<'ast>, env: &mut FlowEnv) {
        let Some(name) = callee_identifier(&call.callee) else {
            return;
        };
        if !env.merge_helpers.contains(name) {
            return;
        }
        let Some(target_name) = call
            .arguments
            .first()
            .and_then(argument_expression)
            .and_then(expression_identifier)
        else {
            return;
        };
        let mut merged = ObjectTargets::default();
        for source in call
            .arguments
            .iter()
            .skip(1)
            .filter_map(argument_expression)
        {
            if let Some(targets) = self.object_targets_from_expression(source, env) {
                merged.merge(targets);
            }
        }
        if merged.is_empty() {
            return;
        }
        env.objects
            .entry(target_name.to_string())
            .or_default()
            .merge(merged);
    }

    /// BDD test globals (`describe`/`it`/`before*`/`after*`, …) invoke their
    /// function argument — like a promise handler, the framework calls the
    /// callback even though there is no syntactic call. Walk each function-argument
    /// body as an invoked callback so its inner calls resolve and the callback
    /// becomes reachable (a `Token` caller). Mocha's `test.js`/
    /// `test-with-hook.js` are otherwise 0-TP — every edge lives inside an
    /// `it(...)`/`describe(...)` callback.
    fn collect_test_framework_callbacks(
        &mut self,
        call: &'ast CallExpression<'ast>,
        env: &mut FlowEnv,
    ) {
        let callee = match &call.callee {
            // Plain `describe(...)` / `it(...)`.
            Expression::Identifier(identifier) => identifier.name.as_str(),
            // `describe.only(...)` / `it.skip(...)` etc.
            Expression::StaticMemberExpression(member) => {
                let Some(base) = expression_identifier(&member.object) else {
                    return;
                };
                base
            }
            _ => return,
        };
        if !is_test_framework_global(callee) {
            return;
        }
        for argument in &call.arguments {
            let Some(expression) = argument_expression(argument) else {
                continue;
            };
            if matches!(
                expression,
                Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_)
            ) && let Some(callback) = self.function_for_expression(expression)
            {
                self.collect_callback_value_flows(callback, expression, Vec::new(), env);
            }
        }
    }

    /// Property name(s) written by `this.X = …` (static) or `this[k] = …`
    /// (computed) where `k` resolves to a constant string (`this[name]` with
    /// `const name = "bar"`, or `this["f" + "oo"]`). Resolves the computed key via
    /// `computed_member_property_names` (const bindings + string concatenation),
    /// so a later `instance.bar()` / `instance.foo()` dispatches to the value.
    fn this_assignment_property_names(
        &self,
        target: &'ast AssignmentTarget<'ast>,
        env: &FlowEnv,
    ) -> Vec<String> {
        match target {
            AssignmentTarget::StaticMemberExpression(member)
                if matches!(&member.object, Expression::ThisExpression(_)) =>
            {
                vec![member.property.name.to_string()]
            }
            AssignmentTarget::ComputedMemberExpression(member)
                if matches!(&member.object, Expression::ThisExpression(_)) =>
            {
                self.computed_member_property_names(&member.expression, env)
            }
            _ => Vec::new(),
        }
    }

    fn collect_native_object_call(&mut self, call: &'ast CallExpression<'ast>, env: &mut FlowEnv) {
        match callee_text(&call.callee).as_deref() {
            Some("Object.assign") => {
                let Some(target_name) = call
                    .arguments
                    .first()
                    .and_then(argument_expression)
                    .and_then(expression_identifier)
                else {
                    return;
                };
                let mut merged = ObjectTargets::default();
                for source in call
                    .arguments
                    .iter()
                    .skip(1)
                    .filter_map(argument_expression)
                {
                    if let Some(targets) = self.object_targets_from_expression(source, env) {
                        merged.merge(targets);
                    }
                }
                if !merged.is_empty()
                    && let Some(target) = env.objects.get_mut(target_name)
                {
                    target.merge(merged);
                }
            }
            Some("Object.defineProperty") => {
                let Some(target_name) = call
                    .arguments
                    .first()
                    .and_then(argument_expression)
                    .and_then(expression_identifier)
                else {
                    return;
                };
                let Some(property) = call
                    .arguments
                    .get(1)
                    .and_then(argument_expression)
                    .and_then(|expression| self.constant_string_from_expression(expression, env))
                else {
                    return;
                };
                let descriptor = call
                    .arguments
                    .get(2)
                    .and_then(argument_expression)
                    .and_then(|expression| self.object_targets_from_expression(expression, env));
                if let (Some(target), Some(descriptor)) =
                    (env.objects.get_mut(target_name), descriptor)
                {
                    copy_descriptor_to_property(target, property, &descriptor);
                }
            }
            Some("Object.setPrototypeOf") => {
                // `Object.setPrototypeOf(obj, proto)` makes `obj` inherit `proto`'s
                // members, so `obj.m()` dispatches to the prototype.
                let Some(target_name) = call
                    .arguments
                    .first()
                    .and_then(argument_expression)
                    .and_then(expression_identifier)
                else {
                    return;
                };
                let proto = call
                    .arguments
                    .get(1)
                    .and_then(argument_expression)
                    .and_then(|expression| self.object_targets_from_expression(expression, env));
                if let (Some(target), Some(proto)) = (env.objects.get_mut(target_name), proto) {
                    target.merge(proto.clone());
                    env.object_prototypes.insert(target_name.to_string(), proto);
                }
            }
            Some("Object.defineProperties") => {
                let Some(target_name) = call
                    .arguments
                    .first()
                    .and_then(argument_expression)
                    .and_then(expression_identifier)
                else {
                    return;
                };
                let descriptors = call
                    .arguments
                    .get(1)
                    .and_then(argument_expression)
                    .and_then(|expression| self.object_targets_from_expression(expression, env));
                if let (Some(target), Some(descriptors)) =
                    (env.objects.get_mut(target_name), descriptors)
                {
                    apply_descriptor_map(target, &descriptors);
                }
            }
            _ => {}
        }
    }

    fn collect_inline_callee_value_flows(
        &mut self,
        call: &'ast CallExpression<'ast>,
        env: &FlowEnv,
    ) {
        let Some(expression) = expression_as_function(&call.callee) else {
            return;
        };
        let Some(function) = self.function_for_expression(expression) else {
            return;
        };
        let bindings = call
            .arguments
            .iter()
            .enumerate()
            .filter_map(|(index, argument)| {
                argument_expression(argument)
                    .and_then(|expression| self.targets_from_argument_expression(expression, env))
                    .map(|targets| (index, targets))
            })
            .collect::<Vec<_>>();
        self.collect_callback_value_flows(function, expression, bindings, env);
    }

    fn collect_function_parameter_flows(
        &mut self,
        flow: &FunctionFlow<'db, 'ast>,
        call: &'ast CallExpression<'ast>,
        env: &FlowEnv,
    ) {
        let arguments = self.argument_targets_from_call(call, env);
        let object_arguments = self.object_arguments_from_call(call, env);
        let mut callee_env = FlowEnv::default();
        self.bind_invocation_arguments(flow, &arguments, &object_arguments, &mut callee_env);
        self.collect_function_flow_invocation(flow, &mut callee_env);
    }

    /// Resolve a tagged template `tag`…${e1}${e2}`…``. The call edge itself comes
    /// from the MIR call site; here we flow the interpolations into the tag's
    /// parameters (`tag(strings, e1, e2)`, so `e_i` → param `i + 1`) and walk the
    /// tag body so its uses of those parameters resolve.
    fn collect_tagged_template_expression(
        &mut self,
        tagged: &'ast oxc_ast::ast::TaggedTemplateExpression<'ast>,
        owner: &'db FunctionFact,
        env: &mut FlowEnv,
    ) {
        self.collect_expression(&tagged.tag, owner, env);
        for expression in &tagged.quasi.expressions {
            self.collect_expression(expression, owner, env);
        }
        if let Expression::Identifier(identifier) = &tagged.tag
            && let Some(flow) = self
                .function_declarations
                .get(identifier.name.as_str())
                .cloned()
        {
            self.collect_tagged_template_parameter_flows(&flow, tagged, env);
        }
    }

    fn collect_tagged_template_parameter_flows(
        &mut self,
        flow: &FunctionFlow<'db, 'ast>,
        tagged: &'ast oxc_ast::ast::TaggedTemplateExpression<'ast>,
        env: &FlowEnv,
    ) {
        // Argument slot 0 is the implicit `strings` array; interpolations follow.
        let mut arguments = vec![CollectionTargets::default()];
        let mut object_arguments: Vec<Option<ObjectTargets>> = vec![None];
        for expression in &tagged.quasi.expressions {
            arguments.push(
                self.targets_from_argument_expression(expression, env)
                    .unwrap_or_default(),
            );
            object_arguments.push(self.object_targets_from_expression(expression, env));
        }
        let mut callee_env = FlowEnv::default();
        self.bind_invocation_arguments(flow, &arguments, &object_arguments, &mut callee_env);
        self.collect_function_flow_invocation(flow, &mut callee_env);
    }

    /// The callable values a tagged template evaluates to — the tag function's
    /// return value — so `const x = tag`…``; x()` resolves.
    fn tagged_template_return_targets(
        &self,
        tagged: &'ast oxc_ast::ast::TaggedTemplateExpression<'ast>,
        env: &FlowEnv,
    ) -> Option<CollectionTargets> {
        let name = expression_identifier(&tagged.tag)?;
        let flow = self.function_declarations.get(name)?;
        let returned = returned_expression_from_statements(&flow.body.statements)?;
        Some(self.callable_targets_from_expression(returned, env))
    }

    fn collect_receiver_side_effects(
        &mut self,
        object_name: &str,
        targets: &[FunctionId],
        call: &'ast CallExpression<'ast>,
        env: &mut FlowEnv,
    ) {
        let Some(receiver) = env.objects.get(object_name).cloned() else {
            return;
        };
        // `super.m()` inside an object-literal method resolves against the object's
        // prototype. Bind `current_super` to the recorded prototype ONLY for objects
        // given one via `Object.setPrototypeOf` / `__proto__` — not every receiver —
        // so `super` in unrelated object methods stays unresolved (no false edges).
        // (super.js / super3.js `q2.m2(){ super.m1() }` → q1.m1.)
        let saved_super = self.current_super.take();
        let object_super = env.object_prototypes.get(object_name).cloned();
        let mut merged_receiver = receiver.clone();
        for target in targets {
            let Some(flow) = self.function_flows_by_id.get(target).cloned() else {
                continue;
            };
            let mut callee_env = env.clone();
            callee_env.this_object = receiver.clone();
            self.current_super = object_super.clone();
            self.bind_call_arguments_to_flow(&flow, call, env, &mut callee_env);
            for statement in &flow.body.statements {
                self.collect_statement(statement, flow.function, &mut callee_env);
            }
            merged_receiver.merge(callee_env.this_object);
        }
        self.current_super = saved_super;
        env.objects.insert(object_name.to_string(), merged_receiver);
    }

    fn collect_setter_assignment_side_effects(
        &mut self,
        object_name: &str,
        targets: &[FunctionId],
        value: &'ast Expression<'ast>,
        env: &mut FlowEnv,
    ) {
        let Some(receiver) = env.objects.get(object_name).cloned() else {
            return;
        };
        let arguments = vec![
            self.targets_from_argument_expression(value, env)
                .unwrap_or_default(),
        ];
        let object_arguments = vec![self.object_targets_from_expression(value, env)];
        let mut merged_receiver = receiver.clone();

        for target in targets {
            let Some(flow) = self.function_flows_by_id.get(target).cloned() else {
                continue;
            };
            let mut callee_env = env.clone();
            callee_env.this_object = receiver.clone();
            self.bind_invocation_arguments(&flow, &arguments, &object_arguments, &mut callee_env);
            self.collect_function_flow_invocation(&flow, &mut callee_env);
            merged_receiver.merge(callee_env.this_object);
        }

        env.objects.insert(object_name.to_string(), merged_receiver);
    }

    fn collect_getter_return_targets(
        &mut self,
        getters: Vec<FunctionId>,
        receiver: ObjectTargets,
        depth: usize,
    ) -> (CollectionTargets, ObjectTargets) {
        if depth > 8 {
            return (CollectionTargets::default(), receiver);
        }

        let mut returned = CollectionTargets::default();
        let mut merged_receiver = receiver.clone();
        for target in getters {
            let Some(flow) = self.function_flows_by_id.get(&target).cloned() else {
                continue;
            };
            let mut callee_env = FlowEnv {
                this_object: receiver.clone(),
                ..FlowEnv::default()
            };
            returned.extend(self.collect_function_flow_invocation(&flow, &mut callee_env));
            merged_receiver.merge(callee_env.this_object);
        }

        (returned, merged_receiver)
    }

    /// Bind an object-destructuring pattern against a resolved source object,
    /// handling forms the free `bind_object_pattern` cannot (they need the
    /// collector): nested object patterns (`{b: {c: y}}`), getter-valued sources
    /// (`{bar: y}` where `bar` is a getter), and default values used when the
    /// property is absent (`{d: y = () => {}}`).
    fn collect_object_pattern_binding(
        &mut self,
        pattern: &'ast BindingPattern<'ast>,
        source: &ObjectTargets,
        env: &mut FlowEnv,
    ) {
        match pattern {
            BindingPattern::BindingIdentifier(identifier) => {
                env.objects
                    .insert(identifier.name.to_string(), source.clone());
            }
            BindingPattern::ObjectPattern(object) => {
                let mut used = Vec::new();
                for property in &object.properties {
                    let Some(name) = static_property_key(&property.key, property.computed) else {
                        continue;
                    };
                    used.push(name.clone());
                    self.collect_object_pattern_property(&property.value, source, &name, env);
                }
                if let Some(rest) = &object.rest {
                    self.collect_object_pattern_binding(
                        &rest.argument,
                        &source.without_properties(&used),
                        env,
                    );
                }
            }
            BindingPattern::AssignmentPattern(pattern) => {
                self.collect_object_pattern_binding(&pattern.left, source, env);
            }
            _ => {}
        }
    }

    fn collect_object_pattern_property(
        &mut self,
        value: &'ast BindingPattern<'ast>,
        source: &ObjectTargets,
        name: &str,
        env: &mut FlowEnv,
    ) {
        // The pattern target (`value`), unwrapping a `= default` wrapper.
        let target = match value {
            BindingPattern::AssignmentPattern(pattern) => &pattern.left,
            other => other,
        };
        let nested_object = source
            .object_properties
            .get(name)
            .map(|nested| (**nested).clone());
        let mut callables = source.properties.get(name).cloned().unwrap_or_default();
        // Getter-valued source: a `get name()` resolves to its return value.
        if let Some(getters) = source.getter_properties.get(name).cloned() {
            let (returned, _) = self.collect_getter_return_targets(getters, source.clone(), 1);
            callables.extend(returned.all_targets());
        }
        // Default value: used only when the property KEY is absent from the source
        // (JS uses the default iff the property is `undefined`, regardless of whether
        // we could resolve a present value; keying on `callables.is_empty()` instead
        // would bind the dead default for a present-but-unresolved property — a
        // false positive). Jelly's reachability-pruned oracle excludes a present
        // property's dead default.
        let key_present = source.properties.contains_key(name)
            || source.getter_properties.contains_key(name)
            || source.setter_properties.contains_key(name)
            || source.object_properties.contains_key(name)
            || source.collection_properties.contains_key(name);
        if !key_present && let BindingPattern::AssignmentPattern(pattern) = value {
            callables.extend(
                self.callable_targets_from_expression(&pattern.right, env)
                    .all_targets(),
            );
        }
        callables.sort();
        callables.dedup();

        // Nested object pattern: recurse against the nested object's targets.
        if let BindingPattern::ObjectPattern(_) = target
            && let Some(nested) = nested_object
        {
            self.collect_object_pattern_binding(target, &nested, env);
            return;
        }
        if let BindingPattern::BindingIdentifier(identifier) = target {
            if let Some(nested) = &nested_object {
                env.objects
                    .insert(identifier.name.to_string(), nested.clone());
            }
            if !callables.is_empty() {
                env.bindings.insert(
                    identifier.name.to_string(),
                    CollectionTargets {
                        keys: Vec::new(),
                        values: callables,
                        object_values: Vec::new(),
                        ..Default::default()
                    },
                );
            }
            return;
        }
        // Array/other pattern: bind the collected callables positionally.
        bind_collection_pattern(
            target,
            &CollectionTargets {
                keys: Vec::new(),
                values: callables,
                object_values: Vec::new(),
                ..Default::default()
            },
            env,
        );
    }

    fn collect_returned_callable_invocation(
        &mut self,
        object_name: Option<&str>,
        targets: &[FunctionId],
        call: &'ast CallExpression<'ast>,
        receiver: ObjectTargets,
        env: &mut FlowEnv,
    ) {
        let receiver = object_name
            .and_then(|name| env.objects.get(name).cloned())
            .unwrap_or(receiver);
        let arguments = self.argument_targets_from_call(call, env);
        let object_arguments = self.object_arguments_from_call(call, env);
        let mut merged_receiver = receiver.clone();

        for target in targets {
            let Some(flow) = self.function_flows_by_id.get(target).cloned() else {
                continue;
            };
            let mut callee_env = env.clone();
            callee_env.this_object = receiver.clone();
            self.bind_invocation_arguments(&flow, &arguments, &object_arguments, &mut callee_env);
            self.collect_function_flow_invocation(&flow, &mut callee_env);
            merged_receiver.merge(callee_env.this_object);
        }

        if let Some(object_name) = object_name {
            env.objects.insert(object_name.to_string(), merged_receiver);
        }
    }

    fn bind_call_arguments_to_flow(
        &self,
        flow: &FunctionFlow<'db, 'ast>,
        call: &'ast CallExpression<'ast>,
        parent_env: &FlowEnv,
        callee_env: &mut FlowEnv,
    ) {
        let arguments = self.argument_targets_from_call(call, parent_env);
        let object_arguments = self.object_arguments_from_call(call, parent_env);
        self.bind_invocation_arguments(flow, &arguments, &object_arguments, callee_env);
    }

    fn collect_function_prototype_method_call(
        &mut self,
        call: &'ast CallExpression<'ast>,
        owner: &'db FunctionFact,
        env: &FlowEnv,
    ) -> Option<CollectionTargets> {
        let Expression::StaticMemberExpression(member) = &call.callee else {
            return None;
        };
        let method = member.property.name.as_str();
        if !matches!(method, "call" | "apply" | "bind") {
            return None;
        }
        let target_ids = self.function_target_ids_from_expression(&member.object, env);
        if target_ids.is_empty() {
            return None;
        }
        if method == "bind" {
            // `.bind()` is an allocation, not a call: `Function.prototype.bind`
            // returns a NEW bound function without invoking the original. The
            // bound value flows to its binding via
            // `bound_function_targets_from_bind_call`, and the eventual `f()`
            // resolves there. Emitting a call edge at the `.bind()` site itself
            // is a false positive (Jelly models bind as alloc), so return None
            // and let `collect_call_expression` emit nothing here.
            return None;
        }
        let targets = CollectionTargets {
            keys: Vec::new(),
            values: target_ids.clone(),
            object_values: Vec::new(),
            ..Default::default()
        };

        let (this_object, this_callables, arguments, object_arguments) =
            self.native_call_receiver_and_arguments(call, method, env);
        for target in target_ids {
            let Some(flow) = self.function_flows_by_id.get(&target).cloned() else {
                continue;
            };
            let mut callee_env = FlowEnv {
                this_object: this_object.clone(),
                this_callables: this_callables.clone(),
                ..FlowEnv::default()
            };
            self.bind_invocation_arguments(&flow, &arguments, &object_arguments, &mut callee_env);
            self.collect_function_flow_invocation(&flow, &mut callee_env);
        }

        self.emit_call_span_targets(owner, call.span, method, Some(targets.all_targets()));
        self.emit_containing_call_span_targets(
            owner,
            call.span,
            method,
            Some(targets.all_targets()),
        );
        Some(targets)
    }

    fn collect_bound_function_invocation(
        &mut self,
        bound: &BoundFunctionTarget,
        call: &'ast CallExpression<'ast>,
        parent_env: &FlowEnv,
    ) {
        let Some(flow) = self.function_flows_by_id.get(&bound.function).cloned() else {
            return;
        };
        let mut arguments = bound.bound_arguments.clone();
        arguments.extend(self.argument_targets_from_call(call, parent_env));
        let mut object_arguments = vec![None; bound.bound_arguments.len()];
        object_arguments.extend(self.object_arguments_from_call(call, parent_env));
        let mut callee_env = FlowEnv {
            this_object: bound.this_object.clone(),
            this_callables: bound.this_callables.clone(),
            ..FlowEnv::default()
        };
        self.bind_invocation_arguments(&flow, &arguments, &object_arguments, &mut callee_env);
        self.collect_function_flow_invocation(&flow, &mut callee_env);
    }

    fn bind_invocation_arguments(
        &self,
        flow: &FunctionFlow<'db, 'ast>,
        arguments: &[CollectionTargets],
        object_arguments: &[Option<ObjectTargets>],
        callee_env: &mut FlowEnv,
    ) {
        let mut all_arguments = CollectionTargets::default();
        for argument in arguments {
            all_arguments.extend(argument.clone());
        }
        callee_env
            .bindings
            .insert("arguments".to_string(), all_arguments);

        let arg_count = arguments.len().max(object_arguments.len());
        for (index, pattern) in flow.params.iter().enumerate() {
            if let Some(targets) = arguments.get(index) {
                bind_param_pattern(pattern, targets, callee_env);
            }
            if let Some(Some(targets)) = object_arguments.get(index) {
                bind_param_object_pattern(pattern, targets, callee_env);
            }
            // A trailing parameter with no corresponding positional argument is
            // `undefined` for this invocation. Record it so `decide_branch` can
            // fold a null/undefined guard on it (see `FlowEnv::absent`).
            if index >= arg_count
                && let ParamPattern::Binding(name) = pattern
            {
                callee_env.absent.insert(name.clone());
            }
        }
        if let Some(rest) = &flow.rest {
            let mut targets = CollectionTargets::default();
            for argument in arguments.iter().skip(flow.params.len()) {
                targets.extend(argument.clone());
            }
            bind_param_pattern(rest, &targets, callee_env);
        }
    }

    fn collect_function_flow_invocation(
        &mut self,
        flow: &FunctionFlow<'db, 'ast>,
        env: &mut FlowEnv,
    ) -> CollectionTargets {
        // Bound nesting: a self-/mutually-returning function would otherwise recurse
        // forever here (the per-call `depth` is reset to 0 each time a return
        // statement re-enters `callable_return_targets_from_expression`).
        if self.invocation_depth > 16 {
            return CollectionTargets::default();
        }
        self.invocation_depth += 1;
        // Calls in the invoked body belong to `flow.function`, not to a
        // `caller_override` active in an enclosing constructor/field body.
        let saved_override = self.caller_override.take();
        let result = if let Some(expression) = flow.expression_body {
            self.collect_expression(expression, flow.function, env);
            self.callable_return_targets_from_expression(expression, env, 0)
        } else {
            self.collect_invocation_statements(&flow.body.statements, flow.function, env)
        };
        self.caller_override = saved_override;
        self.invocation_depth -= 1;
        result
    }

    /// Bound closures produced by `recv.m()` when `m` returns an **arrow** that
    /// captures `this`: the arrow is bound to `this = recv`, so a later invocation
    /// (`const l = o.foo(); l()` or `o.foo()()`) walks the arrow body with the
    /// captured `this` and emits `this.bar()` -> `o.bar` — but only when the arrow
    /// is actually invoked (no edge for a returned-but-uncalled closure). Restricted
    /// to arrows: a returned `function` expression gets its own `this` at call time.
    fn bound_closures_from_call(
        &self,
        call: &'ast CallExpression<'ast>,
        env: &FlowEnv,
    ) -> Vec<BoundFunctionTarget> {
        let Expression::StaticMemberExpression(member) = &call.callee else {
            return Vec::new();
        };
        if matches!(member.property.name.as_str(), "call" | "apply" | "bind") {
            return Vec::new();
        }
        let Some(receiver) = self.object_targets_from_expression(&member.object, env) else {
            return Vec::new();
        };
        let Some(method_ids) = receiver.properties.get(member.property.name.as_str()) else {
            return Vec::new();
        };
        let mut bound = Vec::new();
        for method_id in method_ids {
            let Some(flow) = self.function_flows_by_id.get(method_id) else {
                continue;
            };
            let returned = flow
                .expression_body
                .or_else(|| returned_expression_from_statements(&flow.body.statements));
            let Some(returned) = returned else {
                continue;
            };
            let returned = match returned {
                Expression::ParenthesizedExpression(inner) => &inner.expression,
                other => other,
            };
            if matches!(returned, Expression::ArrowFunctionExpression(_))
                && let Some(arrow) = self.function_for_expression(returned)
            {
                bound.push(BoundFunctionTarget {
                    function: arrow.id,
                    this_object: receiver.clone(),
                    this_callables: CollectionTargets::default(),
                    bound_arguments: Vec::new(),
                });
            }
        }
        bound
    }

    fn collect_invocation_statements(
        &mut self,
        statements: &'ast oxc_allocator::Vec<'ast, Statement<'ast>>,
        owner: &'db FunctionFact,
        env: &mut FlowEnv,
    ) -> CollectionTargets {
        for statement in statements {
            // Absence-folded guard with an early return: `if (depth == null) { return … }`
            // followed by `return flattenWithDepth(…)`. When the param is known-absent
            // the guard's live arm is taken; if it terminates (return/throw), the rest
            // of the block is dead and must not be walked (so `flattenWithDepth` is
            // never reached). The plain `collect_invocation_statement` only stops on a
            // non-empty *return target*, which misses guards whose live arm returns a
            // non-callable (an array), so handle the termination here.
            if let Statement::IfStatement(if_stmt) = statement
                && let Some(branch) = self.decide_branch(&if_stmt.test, env)
            {
                self.collect_expression(&if_stmt.test, owner, env);
                let live = if branch {
                    Some(&if_stmt.consequent)
                } else {
                    if_stmt.alternate.as_ref()
                };
                if let Some(live_stmt) = live {
                    let returned = self.collect_invocation_statement(live_stmt, owner, env);
                    if !returned.is_empty() {
                        return returned;
                    }
                    if branch_terminates(live_stmt) {
                        return CollectionTargets::default();
                    }
                }
                continue;
            }
            let returned = self.collect_invocation_statement(statement, owner, env);
            if !returned.is_empty() {
                return returned;
            }
        }
        CollectionTargets::default()
    }

    fn collect_invocation_statement(
        &mut self,
        statement: &'ast Statement<'ast>,
        owner: &'db FunctionFact,
        env: &mut FlowEnv,
    ) -> CollectionTargets {
        match statement {
            Statement::ReturnStatement(statement) => {
                if let Some(argument) = &statement.argument {
                    self.collect_expression(argument, owner, env);
                    self.callable_return_targets_from_expression(argument, env, 0)
                } else {
                    CollectionTargets::default()
                }
            }
            Statement::BlockStatement(block) => {
                self.collect_invocation_statements(&block.body, owner, env)
            }
            Statement::IfStatement(statement) => {
                self.collect_expression(&statement.test, owner, env);
                match self.decide_branch(&statement.test, env) {
                    Some(true) => {
                        self.collect_invocation_statement(&statement.consequent, owner, env)
                    }
                    Some(false) => statement
                        .alternate
                        .as_ref()
                        .map(|alternate| self.collect_invocation_statement(alternate, owner, env))
                        .unwrap_or_default(),
                    None => {
                        let mut targets =
                            self.collect_invocation_statement(&statement.consequent, owner, env);
                        if let Some(alternate) = &statement.alternate {
                            targets
                                .extend(self.collect_invocation_statement(alternate, owner, env));
                        }
                        targets
                    }
                }
            }
            _ => {
                self.collect_statement(statement, owner, env);
                CollectionTargets::default()
            }
        }
    }

    fn callable_return_targets_from_expression(
        &mut self,
        expression: &'ast Expression<'ast>,
        env: &FlowEnv,
        depth: usize,
    ) -> CollectionTargets {
        if depth > 8 {
            return CollectionTargets::default();
        }
        match expression {
            Expression::CallExpression(call) => {
                self.callable_return_targets_from_call(call, env, depth + 1)
            }
            Expression::StaticMemberExpression(member) => {
                if let Some(object) = self.object_targets_from_expression(&member.object, env)
                    && let Some(getters) = object
                        .getter_properties
                        .get(member.property.name.as_str())
                        .cloned()
                {
                    return self.callable_return_targets_from_function_ids(
                        getters,
                        Vec::new(),
                        Vec::new(),
                        object,
                        CollectionTargets::default(),
                        depth + 1,
                    );
                }
                self.callable_targets_from_expression(expression, env)
            }
            Expression::ComputedMemberExpression(member) => {
                if let Some(object) = self.object_targets_from_expression(&member.object, env)
                    && let Some(property) =
                        self.constant_string_from_expression(&member.expression, env)
                    && let Some(getters) = object.getter_properties.get(&property).cloned()
                {
                    return self.callable_return_targets_from_function_ids(
                        getters,
                        Vec::new(),
                        Vec::new(),
                        object,
                        CollectionTargets::default(),
                        depth + 1,
                    );
                }
                self.callable_targets_from_expression(expression, env)
            }
            Expression::ParenthesizedExpression(expression) => {
                self.callable_return_targets_from_expression(&expression.expression, env, depth)
            }
            _ => self.callable_targets_from_expression(expression, env),
        }
    }

    fn callable_return_targets_from_call(
        &mut self,
        call: &'ast CallExpression<'ast>,
        env: &FlowEnv,
        depth: usize,
    ) -> CollectionTargets {
        if depth > 8 {
            return CollectionTargets::default();
        }
        if let Some(name) = callee_identifier(&call.callee) {
            let mut targets = CollectionTargets::default();
            if let Some(flow) = self.function_declarations.get(name).cloned() {
                let arguments = self.argument_targets_from_call(call, env);
                let object_arguments = self.object_arguments_from_call(call, env);
                let mut callee_env = FlowEnv::default();
                self.bind_invocation_arguments(
                    &flow,
                    &arguments,
                    &object_arguments,
                    &mut callee_env,
                );
                targets.extend(self.collect_function_flow_invocation(&flow, &mut callee_env));
            }
            if let Some(bound_targets) = env.bound_functions.get(name).cloned() {
                for bound in bound_targets {
                    targets.extend(self.callable_return_targets_from_bound_function(
                        &bound,
                        call,
                        env,
                        depth + 1,
                    ));
                }
            }
            return targets;
        }

        if let Expression::ThisExpression(_) = &call.callee {
            return self.callable_return_targets_from_function_ids(
                env.this_callables.all_targets(),
                self.argument_targets_from_call(call, env),
                self.object_arguments_from_call(call, env),
                env.this_object.clone(),
                env.this_callables.clone(),
                depth + 1,
            );
        }

        if let Expression::StaticMemberExpression(member) = &call.callee {
            let method = member.property.name.as_str();
            // Regular method call `recv.m()` whose body returns a value: invoke it
            // with `this` bound to the receiver and propagate the return value
            // (e.g. `x.p()` where `p` returns `this.q`).
            if !matches!(method, "call" | "apply" | "bind")
                && let Some(receiver) = self.object_targets_from_expression(&member.object, env)
                && let Some(method_targets) = receiver.properties.get(method).cloned()
            {
                let result = self.callable_return_targets_from_function_ids(
                    method_targets,
                    self.argument_targets_from_call(call, env),
                    self.object_arguments_from_call(call, env),
                    receiver,
                    CollectionTargets::default(),
                    depth + 1,
                );
                if !result.is_empty() {
                    return result;
                }
            }
            if matches!(method, "call" | "apply") {
                let target_ids = self.function_target_ids_from_expression(&member.object, env);
                let (this_object, this_callables, arguments, object_arguments) =
                    self.native_call_receiver_and_arguments(call, method, env);
                return self.callable_return_targets_from_function_ids(
                    target_ids,
                    arguments,
                    object_arguments,
                    this_object,
                    this_callables,
                    depth + 1,
                );
            }
            if method == "bind" {
                return CollectionTargets {
                    keys: Vec::new(),
                    values: self.function_target_ids_from_expression(&member.object, env),
                    object_values: Vec::new(),
                    ..Default::default()
                };
            }
        }

        CollectionTargets::default()
    }

    fn callable_return_targets_from_bound_function(
        &mut self,
        bound: &BoundFunctionTarget,
        call: &'ast CallExpression<'ast>,
        env: &FlowEnv,
        depth: usize,
    ) -> CollectionTargets {
        let mut arguments = bound.bound_arguments.clone();
        arguments.extend(self.argument_targets_from_call(call, env));
        let mut object_arguments = vec![None; bound.bound_arguments.len()];
        object_arguments.extend(self.object_arguments_from_call(call, env));
        self.callable_return_targets_from_function_ids(
            vec![bound.function],
            arguments,
            object_arguments,
            bound.this_object.clone(),
            bound.this_callables.clone(),
            depth + 1,
        )
    }

    fn callable_return_targets_from_function_ids(
        &mut self,
        target_ids: Vec<FunctionId>,
        arguments: Vec<CollectionTargets>,
        object_arguments: Vec<Option<ObjectTargets>>,
        this_object: ObjectTargets,
        this_callables: CollectionTargets,
        depth: usize,
    ) -> CollectionTargets {
        if depth > 8 {
            return CollectionTargets::default();
        }
        let mut returned = CollectionTargets::default();
        for target in target_ids {
            let Some(flow) = self.function_flows_by_id.get(&target).cloned() else {
                continue;
            };
            let mut callee_env = FlowEnv {
                this_object: this_object.clone(),
                this_callables: this_callables.clone(),
                ..FlowEnv::default()
            };
            self.bind_invocation_arguments(&flow, &arguments, &object_arguments, &mut callee_env);
            returned.extend(self.collect_function_flow_invocation(&flow, &mut callee_env));
        }
        returned
    }

    fn native_call_receiver_and_arguments(
        &self,
        call: &'ast CallExpression<'ast>,
        method: &str,
        env: &FlowEnv,
    ) -> (
        ObjectTargets,
        CollectionTargets,
        Vec<CollectionTargets>,
        Vec<Option<ObjectTargets>>,
    ) {
        let this_expression = call.arguments.first().and_then(argument_expression);
        let this_object = this_expression
            .and_then(|expression| self.object_targets_from_expression(expression, env))
            .unwrap_or_default();
        let this_callables = this_expression
            .map(|expression| self.callable_targets_from_expression(expression, env))
            .unwrap_or_default();
        match method {
            "call" => (
                this_object,
                this_callables,
                self.argument_targets_from_expressions(
                    call.arguments
                        .iter()
                        .skip(1)
                        .filter_map(argument_expression),
                    env,
                ),
                self.object_arguments_from_expressions(
                    call.arguments
                        .iter()
                        .skip(1)
                        .filter_map(argument_expression),
                    env,
                ),
            ),
            "apply" => {
                let argument_list_expression = call.arguments.get(1).and_then(argument_expression);
                let arguments = argument_list_expression
                    .map(|expression| self.apply_argument_targets_from_expression(expression, env))
                    .unwrap_or_default();
                let object_arguments = argument_list_expression
                    .map(|expression| self.apply_object_arguments_from_expression(expression, env))
                    .unwrap_or_default();
                (this_object, this_callables, arguments, object_arguments)
            }
            _ => (this_object, this_callables, Vec::new(), Vec::new()),
        }
    }

    fn bound_function_targets_from_bind_call(
        &self,
        call: &'ast CallExpression<'ast>,
        env: &FlowEnv,
    ) -> Option<Vec<BoundFunctionTarget>> {
        let Expression::StaticMemberExpression(member) = &call.callee else {
            return None;
        };
        if member.property.name != "bind" {
            return None;
        }
        let target_ids = self.function_target_ids_from_expression(&member.object, env);
        if target_ids.is_empty() {
            return None;
        }
        let this_expression = call.arguments.first().and_then(argument_expression);
        let this_object = this_expression
            .and_then(|expression| self.object_targets_from_expression(expression, env))
            .unwrap_or_default();
        let this_callables = this_expression
            .map(|expression| self.callable_targets_from_expression(expression, env))
            .unwrap_or_default();
        let bound_arguments = self.argument_targets_from_expressions(
            call.arguments
                .iter()
                .skip(1)
                .filter_map(argument_expression),
            env,
        );
        Some(
            target_ids
                .into_iter()
                .map(|function| BoundFunctionTarget {
                    function,
                    this_object: this_object.clone(),
                    this_callables: this_callables.clone(),
                    bound_arguments: bound_arguments.clone(),
                })
                .collect(),
        )
    }

    fn argument_targets_from_call(
        &self,
        call: &'ast CallExpression<'ast>,
        env: &FlowEnv,
    ) -> Vec<CollectionTargets> {
        self.argument_targets_from_expressions(
            call.arguments.iter().filter_map(argument_expression),
            env,
        )
    }

    fn object_arguments_from_call(
        &self,
        call: &'ast CallExpression<'ast>,
        env: &FlowEnv,
    ) -> Vec<Option<ObjectTargets>> {
        self.object_arguments_from_expressions(
            call.arguments.iter().filter_map(argument_expression),
            env,
        )
    }

    fn argument_targets_from_expressions<I>(
        &self,
        expressions: I,
        env: &FlowEnv,
    ) -> Vec<CollectionTargets>
    where
        I: Iterator<Item = &'ast Expression<'ast>>,
    {
        expressions
            .map(|expression| {
                self.targets_from_argument_expression(expression, env)
                    .unwrap_or_default()
            })
            .collect()
    }

    fn object_arguments_from_expressions<I>(
        &self,
        expressions: I,
        env: &FlowEnv,
    ) -> Vec<Option<ObjectTargets>>
    where
        I: Iterator<Item = &'ast Expression<'ast>>,
    {
        expressions
            .map(|expression| self.object_targets_from_expression(expression, env))
            .collect()
    }

    fn apply_argument_targets_from_expression(
        &self,
        expression: &'ast Expression<'ast>,
        env: &FlowEnv,
    ) -> Vec<CollectionTargets> {
        match expression {
            Expression::Identifier(identifier) => env
                .bindings
                .get(identifier.name.as_str())
                .map(CollectionTargets::argument_list)
                .unwrap_or_default(),
            Expression::ArrayExpression(array) => array
                .elements
                .iter()
                .filter_map(array_element_expression)
                .map(|expression| {
                    self.targets_from_argument_expression(expression, env)
                        .unwrap_or_default()
                })
                .collect(),
            Expression::ParenthesizedExpression(expression) => {
                self.apply_argument_targets_from_expression(&expression.expression, env)
            }
            _ => self
                .collection_targets_from_expression(expression, env)
                .map(|targets| targets.argument_list())
                .unwrap_or_default(),
        }
    }

    fn apply_object_arguments_from_expression(
        &self,
        expression: &'ast Expression<'ast>,
        env: &FlowEnv,
    ) -> Vec<Option<ObjectTargets>> {
        match expression {
            Expression::Identifier(identifier) => env
                .bindings
                .get(identifier.name.as_str())
                .map(|targets| {
                    targets
                        .argument_list()
                        .into_iter()
                        .map(|argument| argument.singular_object())
                        .collect()
                })
                .unwrap_or_default(),
            Expression::ArrayExpression(array) => array
                .elements
                .iter()
                .filter_map(array_element_expression)
                .map(|expression| self.object_targets_from_expression(expression, env))
                .collect(),
            Expression::ParenthesizedExpression(expression) => {
                self.apply_object_arguments_from_expression(&expression.expression, env)
            }
            _ => Vec::new(),
        }
    }

    fn function_target_ids_from_expression(
        &self,
        expression: &'ast Expression<'ast>,
        env: &FlowEnv,
    ) -> Vec<FunctionId> {
        let mut targets = self
            .callable_targets_from_expression(expression, env)
            .all_targets();
        if let Some(name) = expression_identifier(expression) {
            if let Some(flow) = self.function_declarations.get(name) {
                targets.push(flow.function.id);
            }
            if let Some(bound_targets) = env.bound_functions.get(name) {
                targets.extend(bound_targets.iter().map(|target| target.function));
            }
        }
        targets.sort();
        targets.dedup();
        targets
    }

    fn targets_from_argument_expression(
        &self,
        expression: &'ast Expression<'ast>,
        env: &FlowEnv,
    ) -> Option<CollectionTargets> {
        if let Some(function) = self.function_for_expression(expression) {
            return Some(CollectionTargets {
                keys: Vec::new(),
                values: vec![function.id],
                object_values: Vec::new(),
                ..Default::default()
            });
        }
        if let Some(name) = expression_identifier(expression)
            && let Some(flow) = self.function_declarations.get(name)
        {
            return Some(CollectionTargets {
                keys: Vec::new(),
                values: vec![flow.function.id],
                object_values: Vec::new(),
                ..Default::default()
            });
        }
        self.collection_targets_from_expression(expression, env)
    }

    fn collect_for_of_bindings(
        &mut self,
        left: &'ast ForStatementLeft<'ast>,
        iterable_targets: &CollectionTargets,
        body_span: oxc_span::Span,
        owner: &'db FunctionFact,
    ) {
        if let ForStatementLeft::VariableDeclaration(variable) = left {
            for declarator in &variable.declarations {
                self.collect_loop_binding_pattern(
                    &declarator.id,
                    iterable_targets,
                    body_span,
                    owner,
                );
            }
        }
    }

    fn collect_loop_binding_pattern(
        &mut self,
        pattern: &'ast BindingPattern<'ast>,
        iterable_targets: &CollectionTargets,
        body_span: oxc_span::Span,
        owner: &'db FunctionFact,
    ) {
        match pattern {
            BindingPattern::BindingIdentifier(identifier) => {
                self.emit_binding_targets(
                    owner,
                    identifier.name.as_str(),
                    body_span,
                    Some(iterable_targets.all_targets()),
                );
            }
            BindingPattern::ArrayPattern(array) => {
                for (index, element) in array.elements.iter().enumerate() {
                    if let Some(binding) = element
                        && let Some(name) = binding_identifier_name(binding)
                    {
                        let targets = if index == 0 {
                            iterable_targets.keys_or_values()
                        } else {
                            iterable_targets.values.clone()
                        };
                        self.emit_binding_targets(owner, &name, body_span, Some(targets));
                    }
                }
            }
            BindingPattern::AssignmentPattern(pattern) => {
                self.collect_loop_binding_pattern(
                    &pattern.left,
                    iterable_targets,
                    body_span,
                    owner,
                );
            }
            _ => {}
        }
    }

    fn collect_callback_parameter_flows(
        &mut self,
        callback: &'db FunctionFact,
        method: &str,
        collection_targets: &CollectionTargets,
        expression: &'ast Expression<'ast>,
    ) {
        let parameter_names = callback_parameter_names(expression);
        for (index, name) in parameter_names.iter().enumerate() {
            let targets = callback_targets_for_parameter(method, index, collection_targets);
            if !targets.is_empty() {
                let span = oxc_span::Span::new(callback.span.start_byte, callback.span.end_byte);
                self.emit_binding_targets(callback, name, span, Some(targets));
            }
        }
    }

    fn collect_this_object_member_flows(
        &mut self,
        callback: &'db FunctionFact,
        object: &ObjectTargets,
    ) {
        let span = oxc_span::Span::new(callback.span.start_byte, callback.span.end_byte);
        for (property, targets) in &object.properties {
            self.emit_member_targets(callback, property, span, Some(targets.clone()));
        }
    }

    fn targets_for_declarator_init(
        &self,
        declarator: &'ast VariableDeclarator<'ast>,
        env: &FlowEnv,
    ) -> Option<CollectionTargets> {
        let init = declarator.init.as_ref()?;
        match init {
            Expression::ArrayExpression(array) => {
                Some(self.array_literal_targets(array, Some(env)))
            }
            Expression::NewExpression(expression) => {
                self.collection_targets_from_new_expression(expression, env)
            }
            Expression::CallExpression(call) => self.collection_targets_from_call(call, env),
            Expression::TaggedTemplateExpression(tagged) => {
                self.tagged_template_return_targets(tagged, env)
            }
            _ => None,
        }
    }

    /// Collection elements produced by `new Array(...)` / `new Set(...)` /
    /// `new Map(...)`, so destructuring (`const [x, y] = new Set([...])`) and
    /// `for-of` over a freshly-built collection resolve their elements.
    fn collection_targets_from_new_expression(
        &self,
        expression: &'ast oxc_ast::ast::NewExpression<'ast>,
        env: &FlowEnv,
    ) -> Option<CollectionTargets> {
        let name = callee_identifier(&expression.callee)?;
        match name {
            "Array" | "Set" => expression
                .arguments
                .first()
                .and_then(argument_expression)
                .and_then(|argument| self.collection_targets_from_expression(argument, env))
                .or_else(|| Some(CollectionTargets::default())),
            "Map" => expression
                .arguments
                .first()
                .and_then(argument_expression)
                .map(|argument| self.map_entries_targets_from_expression(argument, env))
                .or_else(|| Some(CollectionTargets::default())),
            _ => None,
        }
    }

    fn targets_for_iterable(
        &self,
        expression: &'ast Expression<'ast>,
        env: &FlowEnv,
        loop_start: u32,
    ) -> CollectionTargets {
        match expression {
            Expression::Identifier(identifier) => {
                // `for-of` over a named iterator yields the values not yet
                // consumed by preceding `.next()` calls, and never the `return`
                // value (which is `{done: true}`, outside the iteration).
                if let Some(sequence) = env.iterator_sequences.get(identifier.name.as_str()) {
                    let offset =
                        self.next_calls_consumed_before(identifier.name.as_str(), loop_start);
                    return sequence.iterated(offset);
                }
                env.async_iterators
                    .get(identifier.name.as_str())
                    .or_else(|| env.bindings.get(identifier.name.as_str()))
                    .cloned()
                    .unwrap_or_default()
            }
            Expression::CallExpression(call) => {
                // A fresh iterator (`for await (const q of gen())`) iterates all
                // yields from the start, excluding the `return` value.
                if let Some(sequence) = self.generator_sequence_from_call(call, env, 0) {
                    return sequence.iterated(0);
                }
                if let Some(name) = callee_identifier(&call.callee)
                    && let Some(targets) = env.async_generators.get(name)
                {
                    return targets.clone();
                }
                if let Some(targets) = self.generator_targets_from_call(call, env, 0) {
                    return targets;
                }
                if let Some((collection_name, method)) = static_member_call(&call.callee)
                    && let Some(source) = env.bindings.get(collection_name)
                {
                    match method {
                        "values" => CollectionTargets {
                            values: source.values.clone(),
                            object_values: source.object_values.clone(),
                            unknown: source.unknown.clone(),
                            unknown_objects: source.unknown_objects.clone(),
                            dynamic: source.dynamic.clone(),
                            ..Default::default()
                        },
                        "keys" => CollectionTargets {
                            values: source.keys_or_values(),
                            ..Default::default()
                        },
                        "entries" => source.clone(),
                        _ => CollectionTargets::default(),
                    }
                } else {
                    CollectionTargets::default()
                }
            }
            _ => CollectionTargets::default(),
        }
    }

    fn collection_targets_from_expression(
        &self,
        expression: &'ast Expression<'ast>,
        env: &FlowEnv,
    ) -> Option<CollectionTargets> {
        match expression {
            Expression::Identifier(identifier) => {
                env.bindings.get(identifier.name.as_str()).cloned()
            }
            Expression::ArrayExpression(array) => {
                Some(self.array_literal_targets(array, Some(env)))
            }
            Expression::ComputedMemberExpression(member) => {
                let source = self.collection_targets_from_expression(&member.object, env)?;
                let index = numeric_index(&member.expression)?;
                Some(source.value_at(index))
            }
            Expression::StaticMemberExpression(member) => {
                let object = self.object_targets_from_expression(&member.object, env)?;
                object
                    .collection_properties
                    .get(member.property.name.as_str())
                    .cloned()
            }
            Expression::AwaitExpression(expression) => {
                Some(self.fulfilled_targets_from_expression(&expression.argument, env))
            }
            Expression::CallExpression(call) => self.collection_targets_from_call(call, env),
            Expression::NewExpression(expression) => {
                self.collection_targets_from_new_expression(expression, env)
            }
            Expression::ParenthesizedExpression(expression) => {
                self.collection_targets_from_expression(&expression.expression, env)
            }
            _ => None,
        }
    }

    /// If `call` is a static `require("literal")` or `import("literal")` whose
    /// specifier resolves (via the kernel module graph) to a module we have a
    /// converged export summary for, return that summary.
    fn require_summary(
        &self,
        call: &'ast CallExpression<'ast>,
    ) -> Option<&'env ModuleExportSummary> {
        let specifier = self.require_specifier(call)?;
        let target = self
            .resolution_map
            .get(&(self.file.id, specifier.to_string()))?;
        self.module_summaries.get(target)
    }

    /// The merged return-value summary of the function(s) this call resolves to,
    /// looked up cross-file by `FunctionId`. `None` if the callee resolves to no
    /// summarized function (the common case — most calls are not factory calls).
    fn return_summary_for_call(
        &self,
        call: &'ast CallExpression<'ast>,
        env: &FlowEnv,
    ) -> Option<ModuleExportSummary> {
        if self.function_returns.is_empty() {
            return None;
        }
        let callees = self.callee_function_ids(&call.callee, env);
        let mut merged = ModuleExportSummary::default();
        let mut found = false;
        for callee in callees {
            if let Some(summary) = self.function_returns.get(&callee) {
                merged.object.merge(summary.object.clone());
                merged.callables.extend(summary.callables.clone());
                found = true;
            }
        }
        found.then_some(merged)
    }

    /// The `FunctionId`s a callee expression resolves to: a bound identifier
    /// (`express` → the required `createApplication`), a local function
    /// declaration, or a resolved member (`lib.make`).
    fn callee_function_ids(
        &self,
        callee: &'ast Expression<'ast>,
        env: &FlowEnv,
    ) -> Vec<FunctionId> {
        match callee {
            Expression::Identifier(identifier) => {
                let name = identifier.name.as_str();
                let mut ids = env
                    .bindings
                    .get(name)
                    .map(CollectionTargets::all_targets)
                    .unwrap_or_default();
                if let Some(flow) = self.function_declarations.get(name) {
                    ids.push(flow.function.id);
                }
                ids
            }
            Expression::StaticMemberExpression(member) => self
                .object_targets_from_expression(&member.object, env)
                .and_then(|object| object.resolve_call_property(member.property.name.as_str()))
                .unwrap_or_default(),
            Expression::ParenthesizedExpression(inner) => {
                self.callee_function_ids(&inner.expression, env)
            }
            _ => Vec::new(),
        }
    }

    /// TypeScript/Babel emit interop wrappers around `require(...)` that pass the
    /// module value through (`__importDefault`, `__importStar`,
    /// `_interopRequireDefault`, `_interopRequireWildcard`). For call-graph
    /// purposes they are identity on the module's export shape, so return the
    /// wrapped argument expression for the caller to evaluate.
    fn commonjs_interop_argument(
        &self,
        call: &'ast CallExpression<'ast>,
    ) -> Option<&'ast Expression<'ast>> {
        let name = callee_identifier(&call.callee)?;
        if matches!(
            name,
            "__importDefault"
                | "__importStar"
                | "_interopRequireDefault"
                | "_interopRequireWildcard"
        ) {
            call.arguments.first().and_then(argument_expression)
        } else {
            None
        }
    }

    /// The object shape a default-interop helper produces from the module it
    /// wraps: `mod && mod.__esModule ? mod : { default: mod }`.
    ///
    /// Two branches, distinguished by the wrapped module's observed shape rather
    /// than by the helper's name:
    /// - a module that already exposes `default` (`exports.__esModule = true;
    ///   exports.default = …`) is ESM-shaped and passes through unchanged;
    /// - an unmarked CommonJS module whose *value* is callable
    ///   (`module.exports = fn`) is wrapped, so `helper.default` reaches it.
    ///
    /// A plain property-bag CommonJS module has no callable module value to
    /// wrap and keeps its own members. `__importStar` shares this shape: it
    /// preserves the namespace's members and adds `default`.
    fn commonjs_interop_object(
        &self,
        inner: &'ast Expression<'ast>,
        env: &FlowEnv,
    ) -> Option<ObjectTargets> {
        let wrapped = self.object_targets_from_expression(inner, env);
        if wrapped.as_ref().is_some_and(|object| {
            object.properties.contains_key("default")
                || object.object_properties.contains_key("default")
        }) {
            return wrapped;
        }
        let module_value = self
            .collection_targets_from_expression(inner, env)
            .map(|targets| targets.all_targets())
            .unwrap_or_default();
        if module_value.is_empty() {
            return wrapped;
        }
        let mut object = wrapped.unwrap_or_default();
        for target in module_value {
            object.add_property_target("default".to_string(), target);
        }
        Some(object)
    }

    /// Callable branches of a guarded initializer — `a || function () {}`,
    /// `a && function () {}`, `cond ? a : b`. Only a branch that *is* a literal
    /// function/class expression has a declaration to point at; an unknown
    /// member load contributes nothing. Both branches are unioned: either can
    /// be taken at runtime.
    fn guarded_callable_functions(&self, expression: &'ast Expression<'ast>) -> Vec<FunctionId> {
        match expression {
            Expression::LogicalExpression(logical) => {
                let mut targets = self.guarded_callable_functions(&logical.left);
                targets.extend(self.guarded_callable_functions(&logical.right));
                targets
            }
            Expression::ConditionalExpression(conditional) => {
                let mut targets = self.guarded_callable_functions(&conditional.consequent);
                targets.extend(self.guarded_callable_functions(&conditional.alternate));
                targets
            }
            Expression::ParenthesizedExpression(inner) => {
                self.guarded_callable_functions(&inner.expression)
            }
            Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_) => self
                .function_for_expression(expression)
                .map(|function| vec![function.id])
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// Extract the static string specifier of a `require("x")` call, if present.
    fn require_specifier(&self, call: &'ast CallExpression<'ast>) -> Option<&'ast str> {
        let is_require = matches!(&call.callee, Expression::Identifier(identifier) if identifier.name == "require");
        if !is_require {
            return None;
        }
        match call.arguments.first().and_then(argument_expression) {
            Some(Expression::StringLiteral(literal)) => Some(literal.value.as_str()),
            _ => None,
        }
    }

    fn collection_targets_from_call(
        &self,
        call: &'ast CallExpression<'ast>,
        env: &FlowEnv,
    ) -> Option<CollectionTargets> {
        if let Some(summary) = self.require_summary(call)
            && !summary.callables.is_empty()
        {
            return Some(summary.callables.clone());
        }
        if let Some(inner) = self.commonjs_interop_argument(call) {
            return self.collection_targets_from_expression(inner, env);
        }
        if let Some(summary) = self.return_summary_for_call(call, env)
            && !summary.callables.is_empty()
        {
            return Some(summary.callables);
        }
        if callee_text(&call.callee).as_deref() == Some("Array.from") {
            self.array_from_targets(call, env)
        } else if callee_text(&call.callee).as_deref() == Some("Array.of") {
            Some(self.array_of_targets(call, env))
        } else if let Some((collection_name, method)) = static_member_call(&call.callee) {
            let source = env.bindings.get(collection_name)?;
            match method {
                "concat" => Some(self.concat_targets(source, call, env)),
                // Whole-array transforms preserve the element shape: numeric cells
                // stay index-ordered and the `%ARRAY_UNKNOWN` bucket carries through
                // so a downstream `for…of` / callback over the result still sees the
                // dynamically-appended elements.
                "values" | "slice" | "splice" | "filter" | "map" | "flatMap" | "flat" => {
                    Some(CollectionTargets {
                        values: source.values.clone(),
                        object_values: source.object_values.clone(),
                        unknown: source.unknown.clone(),
                        unknown_objects: source.unknown_objects.clone(),
                        dynamic: source.dynamic.clone(),
                        ..Default::default()
                    })
                }
                "keys" => Some(CollectionTargets {
                    values: source.keys_or_values(),
                    ..Default::default()
                }),
                "entries" => Some(source.clone()),
                // `pop`/`shift`/`find` return a dynamically-selected element, which
                // matches Jelly's `%ARRAY_UNKNOWN` read (the appended bucket), NOT
                // every numeric cell.
                "pop" | "shift" | "find" => Some(CollectionTargets {
                    values: source.unknown.clone(),
                    object_values: source.unknown_objects.clone(),
                    ..Default::default()
                }),
                // `arr.at(i)` is a specific-index read: a constant index resolves to
                // exactly that numeric cell (`value_at`), so `arr.at(0)` does not
                // smear across every element. A non-constant index falls back to the
                // whole element set.
                "at" => Some(
                    call.arguments
                        .first()
                        .and_then(argument_expression)
                        .and_then(numeric_index)
                        .map(|index| source.value_at(index))
                        .unwrap_or_else(|| CollectionTargets {
                            values: source.elements(),
                            ..Default::default()
                        }),
                ),
                _ => None,
            }
        } else {
            None
        }
    }

    fn object_targets_from_expression(
        &self,
        expression: &'ast Expression<'ast>,
        env: &FlowEnv,
    ) -> Option<ObjectTargets> {
        match expression {
            Expression::Identifier(identifier) => {
                env.objects.get(identifier.name.as_str()).cloned()
            }
            Expression::ThisExpression(_) => Some(env.this_object.clone()),
            Expression::ObjectExpression(object) => {
                Some(self.object_literal_targets(object, Some(env)))
            }
            Expression::ComputedMemberExpression(member) => {
                let source = self.collection_targets_from_expression(&member.object, env)?;
                let index = numeric_index(&member.expression)?;
                source.object_at(index)
            }
            Expression::NewExpression(expression) => {
                let name = callee_identifier(&expression.callee)?;
                // A variable may hold a class flowed through it (`var a = f(); new a()`);
                // resolve the binding to its class key, else use the name directly.
                let key = env
                    .class_bindings
                    .get(name)
                    .map(String::as_str)
                    .unwrap_or(name);
                let arguments = expression
                    .arguments
                    .iter()
                    .filter_map(argument_expression)
                    .collect::<Vec<_>>();
                self.class_instance_targets(key, &arguments, env)
            }
            Expression::CallExpression(call) => self.object_targets_from_call(call, env),
            Expression::StaticMemberExpression(member) => {
                let object = self.object_targets_from_expression(&member.object, env)?;
                object
                    .object_properties
                    .get(member.property.name.as_str())
                    .map(|targets| (**targets).clone())
            }
            Expression::ParenthesizedExpression(expression) => {
                self.object_targets_from_expression(&expression.expression, env)
            }
            // `a || b` / `a ?? b`: a definite (truthy / non-null) `a` short-circuits
            // to `a`, matching how `(o1 || o2).f()` resolves to `o1` only; fall back
            // to `b` when `a` does not resolve. `a && b` is symmetric (`b` wins).
            Expression::LogicalExpression(logical) => {
                let (first, second) = match logical.operator {
                    LogicalOperator::And => (&logical.right, &logical.left),
                    LogicalOperator::Or | LogicalOperator::Coalesce => {
                        (&logical.left, &logical.right)
                    }
                };
                self.object_targets_from_expression(first, env)
                    .or_else(|| self.object_targets_from_expression(second, env))
            }
            // `cond ? a : b` can take either branch, so the result is their union.
            Expression::ConditionalExpression(conditional) => {
                self.merged_object_targets([&conditional.consequent, &conditional.alternate], env)
            }
            _ => None,
        }
    }

    /// Union the object shapes of several alternative expressions (the branches of
    /// a logical/conditional expression). `None` only if no branch resolves.
    fn merged_object_targets(
        &self,
        expressions: [&'ast Expression<'ast>; 2],
        env: &FlowEnv,
    ) -> Option<ObjectTargets> {
        let mut merged = ObjectTargets::default();
        let mut found = false;
        for expression in expressions {
            if let Some(object) = self.object_targets_from_expression(expression, env) {
                merged.merge(object);
                found = true;
            }
        }
        found.then_some(merged)
    }

    /// Object returned by a call to a same-file function declaration that builds
    /// and returns an object (`function make() { const x = {...}; return x; }`),
    /// for `const {a, b} = make()` destructuring. Walks the callee body to build
    /// the returned object's shape — the read-only `object_targets_from_expression`
    /// cannot, since the object is assembled across statements. Bounded by
    /// `invocation_depth` (shared with the callable-return cycle).
    fn object_targets_from_local_function_call(
        &mut self,
        init: &'ast Expression<'ast>,
        env: &FlowEnv,
    ) -> Option<ObjectTargets> {
        let Expression::CallExpression(call) = init else {
            return None;
        };
        let name = callee_identifier(&call.callee)?;
        let flow = self.function_declarations.get(name).cloned()?;
        if self.invocation_depth > 16 {
            return None;
        }
        let arguments = self.argument_targets_from_call(call, env);
        let object_arguments = self.object_arguments_from_call(call, env);
        let mut callee_env = FlowEnv::default();
        self.bind_invocation_arguments(&flow, &arguments, &object_arguments, &mut callee_env);
        self.invocation_depth += 1;
        let saved_override = self.caller_override.take();
        for statement in &flow.body.statements {
            self.collect_statement(statement, flow.function, &mut callee_env);
        }
        let result = returned_expression_from_statements(&flow.body.statements)
            .and_then(|expression| self.object_targets_from_expression(expression, &callee_env));
        self.caller_override = saved_override;
        self.invocation_depth -= 1;
        result
    }

    fn object_targets_from_call(
        &self,
        call: &'ast CallExpression<'ast>,
        env: &FlowEnv,
    ) -> Option<ObjectTargets> {
        if let Some(summary) = self.require_summary(call)
            && !summary.object.is_empty()
        {
            return Some(summary.object.clone());
        }
        if let Some(inner) = self.commonjs_interop_argument(call) {
            return self.commonjs_interop_object(inner, env);
        }
        // A call to a function summarized elsewhere (`const app = express()`):
        // seed the object shape that function returns (the `app` object that
        // `createApplication` assembles via `mixin`).
        if let Some(object) = self.return_summary_for_call(call, env)
            && !object.object.is_empty()
        {
            return Some(object.object);
        }
        match callee_text(&call.callee).as_deref() {
            Some("Object.assign") => {
                // `Object.assign(target, ...sources)` returns the target with every
                // source's own properties copied in, so `const o = Object.assign({},
                // a, b)` resolves `o`'s members. (The in-place mutation of a *named*
                // target is handled separately in `collect_native_object_call`.)
                let mut merged = ObjectTargets::default();
                for argument in call.arguments.iter().filter_map(argument_expression) {
                    if let Some(object) = self.object_targets_from_expression(argument, env) {
                        merged.merge(object);
                    }
                }
                return Some(merged);
            }
            Some("Object.create") => {
                // `Object.create(proto, descriptors)`: inherit the prototype's
                // properties (first argument), then apply the optional descriptor
                // map (second argument, same shape as `defineProperties`).
                let mut target = ObjectTargets::default();
                if let Some(proto) = call
                    .arguments
                    .first()
                    .and_then(argument_expression)
                    .and_then(|expression| self.object_targets_from_expression(expression, env))
                {
                    target.merge(proto);
                }
                if let Some(descriptors) = call
                    .arguments
                    .get(1)
                    .and_then(argument_expression)
                    .and_then(|expression| self.object_targets_from_expression(expression, env))
                {
                    apply_descriptor_map(&mut target, &descriptors);
                }
                return Some(target);
            }
            Some("Object.getOwnPropertyDescriptor") => {
                let source = call
                    .arguments
                    .first()
                    .and_then(argument_expression)
                    .and_then(|expression| self.object_targets_from_expression(expression, env))?;
                let property = call
                    .arguments
                    .get(1)
                    .and_then(argument_expression)
                    .and_then(|expression| self.constant_string_from_expression(expression, env))?;
                return Some(descriptor_for_property(&source, &property));
            }
            Some("Object.getOwnPropertyDescriptors") => {
                return call
                    .arguments
                    .first()
                    .and_then(argument_expression)
                    .and_then(|expression| self.object_targets_from_expression(expression, env))
                    .map(|source| descriptors_for_object(&source));
            }
            _ => {}
        }

        if let Some((iterator_name, "next")) = static_member_call(&call.callee) {
            // Sequence-sensitive: the k-th `.next()` delivers the k-th yield (or
            // the `return` value at the terminal step), not every yield at once.
            if let Some(sequence) = env.iterator_sequences.get(iterator_name)
                && let Some(ordinal) = self.next_call_ordinal(call)
            {
                return Some(iterator_result_object(&sequence.result_at(ordinal)));
            }
            if let Some(values) = env.async_iterators.get(iterator_name) {
                return Some(iterator_result_object(values));
            }
        }

        let Expression::StaticMemberExpression(member) = &call.callee else {
            return None;
        };
        let receiver = self.object_targets_from_expression(&member.object, env)?;
        let function_ids = receiver.properties.get(member.property.name.as_str())?;
        let mut returned = ObjectTargets::default();
        let mut found = false;

        for function_id in function_ids {
            let Some(flow) = self.function_flows_by_id.get(function_id) else {
                continue;
            };
            let Some(expression) = returned_expression_from_statements(&flow.body.statements)
            else {
                continue;
            };
            if let Some(object) =
                self.object_targets_from_return_expression(expression, &receiver, env)
            {
                returned.merge(object);
                found = true;
            }
        }

        found.then_some(returned)
    }

    fn object_targets_from_return_expression(
        &self,
        expression: &'ast Expression<'ast>,
        receiver: &ObjectTargets,
        env: &FlowEnv,
    ) -> Option<ObjectTargets> {
        match expression {
            Expression::ThisExpression(_) => Some(receiver.clone()),
            Expression::ObjectExpression(object) => {
                Some(self.object_literal_targets(object, Some(env)))
            }
            Expression::Identifier(identifier) => {
                env.objects.get(identifier.name.as_str()).cloned()
            }
            Expression::CallExpression(call) => self.object_targets_from_call(call, env),
            Expression::ParenthesizedExpression(expression) => {
                self.object_targets_from_return_expression(&expression.expression, receiver, env)
            }
            _ => None,
        }
    }

    fn promise_targets_from_expression(
        &self,
        expression: &'ast Expression<'ast>,
        env: &FlowEnv,
    ) -> Option<PromiseTargets> {
        match expression {
            Expression::Identifier(identifier) => {
                env.promises.get(identifier.name.as_str()).cloned()
            }
            Expression::NewExpression(new_expression)
                if callee_identifier(&new_expression.callee) == Some("Promise") =>
            {
                new_expression
                    .arguments
                    .first()
                    .and_then(argument_expression)
                    .and_then(|executor| self.promise_targets_from_executor(executor, env))
            }
            Expression::CallExpression(call) => self.promise_targets_from_call(call, env),
            Expression::ParenthesizedExpression(expression) => {
                self.promise_targets_from_expression(&expression.expression, env)
            }
            _ => None,
        }
    }

    fn promise_targets_from_call(
        &self,
        call: &'ast CallExpression<'ast>,
        env: &FlowEnv,
    ) -> Option<PromiseTargets> {
        if let Some(name) = callee_identifier(&call.callee)
            && let Some(targets) = env.async_functions.get(name)
        {
            return Some(targets.clone());
        }
        if let Some((iterator_name, "next")) = static_member_call(&call.callee) {
            // Async generator `.next()` returns a promise of the sequenced result
            // object; resolve the k-th `.next()` to the k-th yielded value.
            let result = if let Some(sequence) = env.iterator_sequences.get(iterator_name)
                && let Some(ordinal) = self.next_call_ordinal(call)
            {
                Some(iterator_result_object(&sequence.result_at(ordinal)))
            } else {
                env.async_iterators
                    .get(iterator_name)
                    .map(iterator_result_object)
            };
            if let Some(result) = result {
                return Some(PromiseTargets {
                    fulfilled: CollectionTargets {
                        keys: Vec::new(),
                        values: Vec::new(),
                        object_values: vec![result],
                        ..Default::default()
                    },
                    rejected: CollectionTargets::default(),
                });
            }
        }

        match callee_text(&call.callee).as_deref() {
            Some("Promise.resolve") => Some(PromiseTargets {
                fulfilled: call
                    .arguments
                    .first()
                    .and_then(argument_expression)
                    .map(|argument| self.fulfilled_targets_from_expression(argument, env))
                    .unwrap_or_default(),
                rejected: CollectionTargets::default(),
            }),
            Some("Promise.reject") => Some(PromiseTargets {
                fulfilled: CollectionTargets::default(),
                rejected: call
                    .arguments
                    .first()
                    .and_then(argument_expression)
                    .map(|argument| self.callable_targets_from_expression(argument, env))
                    .unwrap_or_default(),
            }),
            Some("Promise.all") => call
                .arguments
                .first()
                .and_then(argument_expression)
                .map(|argument| self.aggregate_promise_targets(argument, env)),
            Some("Promise.any") | Some("Promise.race") => call
                .arguments
                .first()
                .and_then(argument_expression)
                .map(|argument| {
                    // `Promise.any` rejects with an `AggregateError`, never an
                    // input promise's rejection value; and Jelly does not flow
                    // input rejections into `Promise.race`'s reject handler
                    // either (only the settled fulfillment value is observable).
                    // Keep the aggregated fulfillment targets but drop the
                    // rejections so `.then(_, onRejected)` / `.catch` emit no
                    // false edge to an input's reject callback. (`Promise.all`
                    // does reject with the first input rejection, so it keeps
                    // them above.)
                    let mut targets = self.aggregate_promise_targets(argument, env);
                    targets.rejected = CollectionTargets::default();
                    targets
                }),
            Some("Promise.allSettled") => call
                .arguments
                .first()
                .and_then(argument_expression)
                .map(|argument| self.all_settled_promise_targets(argument, env)),
            _ => self
                .promise_method_source(call, env)
                .map(|(source, method)| {
                    self.promise_method_result_targets(call, method, &source, env)
                }),
        }
    }

    fn promise_targets_from_executor(
        &self,
        executor: &'ast Expression<'ast>,
        env: &FlowEnv,
    ) -> Option<PromiseTargets> {
        let names = callback_parameter_names(executor);
        let resolve_name = names.first()?;
        let reject_name = names.get(1).map(String::as_str).unwrap_or("__reject");
        let mut targets = PromiseTargets::default();
        match executor {
            Expression::ArrowFunctionExpression(function) => {
                if let Some(expression) = function.get_expression() {
                    self.collect_promise_executor_expression(
                        expression,
                        resolve_name,
                        reject_name,
                        env,
                        &mut targets,
                    );
                } else {
                    for statement in &function.body.statements {
                        self.collect_promise_executor_statement(
                            statement,
                            resolve_name,
                            reject_name,
                            env,
                            &mut targets,
                        );
                    }
                }
            }
            Expression::FunctionExpression(function) => {
                if let Some(body) = function.body.as_deref() {
                    for statement in &body.statements {
                        self.collect_promise_executor_statement(
                            statement,
                            resolve_name,
                            reject_name,
                            env,
                            &mut targets,
                        );
                    }
                }
            }
            _ => return None,
        }
        Some(targets)
    }

    fn async_function_promise_targets(
        &self,
        expression: &'ast Expression<'ast>,
        env: &FlowEnv,
    ) -> Option<PromiseTargets> {
        match expression {
            Expression::ArrowFunctionExpression(function) if function.r#async => {
                async_arrow_returned_expression(function)
                    .map(|expression| self.promise_targets_from_async_return(expression, env))
            }
            Expression::FunctionExpression(function) if function.r#async && !function.generator => {
                function
                    .body
                    .as_deref()
                    .and_then(|body| returned_expression_from_statements(&body.statements))
                    .map(|expression| self.promise_targets_from_async_return(expression, env))
            }
            Expression::ParenthesizedExpression(expression) => {
                self.async_function_promise_targets(&expression.expression, env)
            }
            _ => None,
        }
    }

    fn async_generator_yield_targets(
        &self,
        expression: &'ast Expression<'ast>,
        env: &FlowEnv,
    ) -> Option<CollectionTargets> {
        match expression {
            Expression::FunctionExpression(function) if function.r#async && function.generator => {
                function
                    .body
                    .as_deref()
                    .map(|body| self.yield_targets_from_statements(&body.statements, env))
            }
            Expression::ParenthesizedExpression(expression) => {
                self.async_generator_yield_targets(&expression.expression, env)
            }
            _ => None,
        }
    }

    fn generator_iterator_targets(
        &self,
        expression: &'ast Expression<'ast>,
        env: &FlowEnv,
    ) -> Option<CollectionTargets> {
        match expression {
            Expression::CallExpression(call) => self.generator_targets_from_call(call, env, 0),
            Expression::ParenthesizedExpression(expression) => {
                self.generator_iterator_targets(&expression.expression, env)
            }
            _ => None,
        }
    }

    fn generator_targets_from_call(
        &self,
        call: &'ast CallExpression<'ast>,
        env: &FlowEnv,
        depth: usize,
    ) -> Option<CollectionTargets> {
        if depth > 8 {
            return None;
        }
        if let Some(name) = callee_identifier(&call.callee) {
            if let Some(targets) = env.async_generators.get(name) {
                return Some(targets.clone());
            }
            if let Some(flow) = self.function_declarations.get(name)
                && flow.generator
            {
                return Some(self.generator_targets_from_flow(flow, env, depth + 1));
            }
        }
        if let Some((collection_name, method)) = static_member_call(&call.callee)
            && let Some(source) = env.bindings.get(collection_name)
        {
            return match method {
                "values" => Some(CollectionTargets {
                    values: source.values.clone(),
                    object_values: source.object_values.clone(),
                    unknown: source.unknown.clone(),
                    unknown_objects: source.unknown_objects.clone(),
                    dynamic: source.dynamic.clone(),
                    ..Default::default()
                }),
                "keys" => Some(CollectionTargets {
                    values: source.keys_or_values(),
                    ..Default::default()
                }),
                "entries" => Some(source.clone()),
                _ => None,
            };
        }
        if let Expression::StaticMemberExpression(member) = &call.callee
            && let Some(object) = self.object_targets_from_expression(&member.object, env)
            && let Some(function_ids) = object.properties.get(member.property.name.as_str())
        {
            let mut targets = CollectionTargets::default();
            for function_id in function_ids {
                if let Some(flow) = self.function_flows_by_id.get(function_id)
                    && flow.generator
                {
                    targets.extend(self.generator_targets_from_flow(flow, env, depth + 1));
                }
            }
            if !targets.is_empty() {
                return Some(targets);
            }
        }
        None
    }

    fn generator_targets_from_expression(
        &self,
        expression: &'ast Expression<'ast>,
        env: &FlowEnv,
        depth: usize,
    ) -> Option<CollectionTargets> {
        match expression {
            Expression::CallExpression(call) => self.generator_targets_from_call(call, env, depth),
            Expression::Identifier(identifier) => env
                .async_iterators
                .get(identifier.name.as_str())
                .or_else(|| env.async_generators.get(identifier.name.as_str()))
                .cloned(),
            Expression::ParenthesizedExpression(expression) => {
                self.generator_targets_from_expression(&expression.expression, env, depth)
            }
            _ => None,
        }
    }

    fn generator_targets_from_flow(
        &self,
        flow: &FunctionFlow<'db, 'ast>,
        env: &FlowEnv,
        depth: usize,
    ) -> CollectionTargets {
        if depth > 8 {
            return CollectionTargets::default();
        }
        self.yield_targets_from_statements(&flow.body.statements, env)
    }

    fn yield_targets_from_statements(
        &self,
        statements: &'ast oxc_allocator::Vec<'ast, Statement<'ast>>,
        env: &FlowEnv,
    ) -> CollectionTargets {
        let mut targets = CollectionTargets::default();
        for statement in statements {
            targets.extend(self.yield_targets_from_statement(statement, env));
        }
        targets
    }

    fn yield_targets_from_statement(
        &self,
        statement: &'ast Statement<'ast>,
        env: &FlowEnv,
    ) -> CollectionTargets {
        match statement {
            Statement::ExpressionStatement(statement) => {
                self.yield_targets_from_expression(&statement.expression, env)
            }
            Statement::ReturnStatement(statement) => statement
                .argument
                .as_ref()
                .map(|argument| self.callable_targets_from_expression(argument, env))
                .unwrap_or_default(),
            Statement::BlockStatement(block) => {
                self.yield_targets_from_statements(&block.body, env)
            }
            Statement::IfStatement(statement) => {
                let mut targets = self.yield_targets_from_statement(&statement.consequent, env);
                if let Some(alternate) = &statement.alternate {
                    targets.extend(self.yield_targets_from_statement(alternate, env));
                }
                targets
            }
            _ => CollectionTargets::default(),
        }
    }

    fn yield_targets_from_expression(
        &self,
        expression: &'ast Expression<'ast>,
        env: &FlowEnv,
    ) -> CollectionTargets {
        match expression {
            Expression::YieldExpression(yield_expression) => {
                let Some(argument) = &yield_expression.argument else {
                    return CollectionTargets::default();
                };
                if yield_expression.delegate {
                    self.generator_targets_from_expression(argument, env, 0)
                        .or_else(|| self.collection_targets_from_expression(argument, env))
                        .unwrap_or_else(|| self.callable_targets_from_expression(argument, env))
                } else {
                    self.callable_targets_from_expression(argument, env)
                }
            }
            Expression::SequenceExpression(sequence) => {
                let mut targets = CollectionTargets::default();
                for expression in &sequence.expressions {
                    targets.extend(self.yield_targets_from_expression(expression, env));
                }
                targets
            }
            Expression::ParenthesizedExpression(expression) => {
                self.yield_targets_from_expression(&expression.expression, env)
            }
            _ => CollectionTargets::default(),
        }
    }

    // --- Sequence-sensitive generator model (one ordered step per yield) ---

    fn async_generator_sequence_targets(
        &self,
        expression: &'ast Expression<'ast>,
        env: &FlowEnv,
    ) -> Option<GeneratorSequence> {
        match expression {
            Expression::FunctionExpression(function) if function.r#async && function.generator => {
                function
                    .body
                    .as_deref()
                    .map(|body| self.yield_sequence_from_statements(&body.statements, env, 0))
            }
            Expression::ParenthesizedExpression(expression) => {
                self.async_generator_sequence_targets(&expression.expression, env)
            }
            _ => None,
        }
    }

    fn generator_iterator_sequence(
        &self,
        expression: &'ast Expression<'ast>,
        env: &FlowEnv,
    ) -> Option<GeneratorSequence> {
        match expression {
            Expression::CallExpression(call) => self.generator_sequence_from_call(call, env, 0),
            Expression::ParenthesizedExpression(expression) => {
                self.generator_iterator_sequence(&expression.expression, env)
            }
            _ => None,
        }
    }

    fn generator_sequence_from_call(
        &self,
        call: &'ast CallExpression<'ast>,
        env: &FlowEnv,
        depth: usize,
    ) -> Option<GeneratorSequence> {
        if depth > 8 {
            return None;
        }
        if let Some(name) = callee_identifier(&call.callee) {
            if let Some(seq) = env.generator_sequences.get(name) {
                return Some(seq.clone());
            }
            if let Some(flow) = self.function_declarations.get(name)
                && flow.generator
            {
                return Some(self.generator_sequence_from_flow(flow, env, depth + 1));
            }
        }
        if let Expression::StaticMemberExpression(member) = &call.callee
            && let Some(object) = self.object_targets_from_expression(&member.object, env)
            && let Some(function_ids) = object.properties.get(member.property.name.as_str())
        {
            let mut seq = GeneratorSequence::default();
            let mut found = false;
            for function_id in function_ids {
                if let Some(flow) = self.function_flows_by_id.get(function_id)
                    && flow.generator
                {
                    seq.merge(self.generator_sequence_from_flow(flow, env, depth + 1));
                    found = true;
                }
            }
            if found {
                return Some(seq);
            }
        }
        None
    }

    fn generator_sequence_from_flow(
        &self,
        flow: &FunctionFlow<'db, 'ast>,
        env: &FlowEnv,
        depth: usize,
    ) -> GeneratorSequence {
        if depth > 8 {
            return GeneratorSequence::default();
        }
        self.yield_sequence_from_statements(&flow.body.statements, env, depth)
    }

    fn yield_sequence_from_statements(
        &self,
        statements: &'ast oxc_allocator::Vec<'ast, Statement<'ast>>,
        env: &FlowEnv,
        depth: usize,
    ) -> GeneratorSequence {
        let mut seq = GeneratorSequence::default();
        for statement in statements {
            self.collect_yield_sequence_from_statement(statement, env, depth, &mut seq);
        }
        seq
    }

    fn collect_yield_sequence_from_statement(
        &self,
        statement: &'ast Statement<'ast>,
        env: &FlowEnv,
        depth: usize,
        seq: &mut GeneratorSequence,
    ) {
        match statement {
            Statement::ExpressionStatement(statement) => {
                self.collect_yield_sequence_from_expression(&statement.expression, env, depth, seq);
            }
            Statement::VariableDeclaration(variable) => {
                for declarator in &variable.declarations {
                    if let Some(init) = &declarator.init {
                        self.collect_yield_sequence_from_expression(init, env, depth, seq);
                    }
                }
            }
            Statement::ReturnStatement(statement) => {
                if let Some(argument) = &statement.argument {
                    seq.ret
                        .extend(self.callable_targets_from_expression(argument, env));
                }
            }
            Statement::BlockStatement(block) => {
                for inner in &block.body {
                    self.collect_yield_sequence_from_statement(inner, env, depth, seq);
                }
            }
            Statement::IfStatement(statement) => {
                self.collect_yield_sequence_from_statement(&statement.consequent, env, depth, seq);
                if let Some(alternate) = &statement.alternate {
                    self.collect_yield_sequence_from_statement(alternate, env, depth, seq);
                }
            }
            Statement::ForOfStatement(statement) => {
                self.collect_yield_sequence_from_statement(&statement.body, env, depth, seq);
            }
            Statement::ForInStatement(statement) => {
                self.collect_yield_sequence_from_statement(&statement.body, env, depth, seq);
            }
            Statement::ForStatement(statement) => {
                self.collect_yield_sequence_from_statement(&statement.body, env, depth, seq);
            }
            Statement::WhileStatement(statement) => {
                self.collect_yield_sequence_from_statement(&statement.body, env, depth, seq);
            }
            Statement::DoWhileStatement(statement) => {
                self.collect_yield_sequence_from_statement(&statement.body, env, depth, seq);
            }
            _ => {}
        }
    }

    fn collect_yield_sequence_from_expression(
        &self,
        expression: &'ast Expression<'ast>,
        env: &FlowEnv,
        depth: usize,
        seq: &mut GeneratorSequence,
    ) {
        match expression {
            Expression::YieldExpression(yield_expression) => {
                let Some(argument) = &yield_expression.argument else {
                    // `yield;` occupies a position but produces no callable value.
                    seq.steps.push(CollectionTargets::default());
                    return;
                };
                if yield_expression.delegate {
                    self.append_yield_star_steps(argument, env, depth, seq);
                } else {
                    seq.steps
                        .push(self.callable_targets_from_expression(argument, env));
                }
            }
            Expression::AssignmentExpression(assignment) => {
                self.collect_yield_sequence_from_expression(&assignment.right, env, depth, seq);
            }
            Expression::SequenceExpression(sequence) => {
                for inner in &sequence.expressions {
                    self.collect_yield_sequence_from_expression(inner, env, depth, seq);
                }
            }
            Expression::ParenthesizedExpression(inner) => {
                self.collect_yield_sequence_from_expression(&inner.expression, env, depth, seq);
            }
            Expression::AwaitExpression(inner) => {
                self.collect_yield_sequence_from_expression(&inner.argument, env, depth, seq);
            }
            _ => {}
        }
    }

    /// `yield* X` splices X's elements into the sequence as successive steps:
    /// an array literal contributes one step per element, a delegated generator
    /// its own steps, so `.next()` ordinals line up with the flattened order.
    fn append_yield_star_steps(
        &self,
        argument: &'ast Expression<'ast>,
        env: &FlowEnv,
        depth: usize,
        seq: &mut GeneratorSequence,
    ) {
        match argument {
            Expression::ArrayExpression(array) => {
                for element in &array.elements {
                    if let Some(inner) = array_element_expression(element) {
                        seq.steps
                            .push(self.callable_targets_from_expression(inner, env));
                    }
                }
            }
            Expression::CallExpression(call) => {
                if let Some(delegated) = self.generator_sequence_from_call(call, env, depth + 1) {
                    seq.steps.extend(delegated.steps);
                } else if let Some(collection) =
                    self.collection_targets_from_expression(argument, env)
                {
                    seq.steps.push(collection);
                }
            }
            Expression::Identifier(identifier) => {
                if let Some(delegated) = env.iterator_sequences.get(identifier.name.as_str()) {
                    seq.steps.extend(delegated.steps.iter().cloned());
                } else if let Some(collection) =
                    self.collection_targets_from_expression(argument, env)
                {
                    seq.steps.push(collection);
                }
            }
            Expression::ParenthesizedExpression(inner) => {
                self.append_yield_star_steps(&inner.expression, env, depth, seq);
            }
            _ => {
                if let Some(collection) = self.collection_targets_from_expression(argument, env) {
                    seq.steps.push(collection);
                }
            }
        }
    }

    fn promise_targets_from_async_return(
        &self,
        expression: &'ast Expression<'ast>,
        env: &FlowEnv,
    ) -> PromiseTargets {
        if let Some(promise) = self.promise_targets_from_expression(expression, env) {
            return promise;
        }
        PromiseTargets {
            fulfilled: self.callable_targets_from_expression(expression, env),
            rejected: CollectionTargets::default(),
        }
    }

    fn collect_promise_executor_statement(
        &self,
        statement: &'ast Statement<'ast>,
        resolve_name: &str,
        reject_name: &str,
        env: &FlowEnv,
        targets: &mut PromiseTargets,
    ) {
        match statement {
            Statement::ExpressionStatement(statement) => self.collect_promise_executor_expression(
                &statement.expression,
                resolve_name,
                reject_name,
                env,
                targets,
            ),
            Statement::ThrowStatement(statement) => {
                targets
                    .rejected
                    .extend(self.callable_targets_from_expression(&statement.argument, env));
            }
            Statement::BlockStatement(block) => {
                for statement in &block.body {
                    self.collect_promise_executor_statement(
                        statement,
                        resolve_name,
                        reject_name,
                        env,
                        targets,
                    );
                }
            }
            Statement::IfStatement(statement) => {
                self.collect_promise_executor_statement(
                    &statement.consequent,
                    resolve_name,
                    reject_name,
                    env,
                    targets,
                );
                if let Some(alternate) = &statement.alternate {
                    self.collect_promise_executor_statement(
                        alternate,
                        resolve_name,
                        reject_name,
                        env,
                        targets,
                    );
                }
            }
            Statement::SwitchStatement(statement) => {
                for case in &statement.cases {
                    for statement in &case.consequent {
                        self.collect_promise_executor_statement(
                            statement,
                            resolve_name,
                            reject_name,
                            env,
                            targets,
                        );
                    }
                }
            }
            _ => {}
        }
    }

    fn collect_promise_executor_expression(
        &self,
        expression: &'ast Expression<'ast>,
        resolve_name: &str,
        reject_name: &str,
        env: &FlowEnv,
        targets: &mut PromiseTargets,
    ) {
        match expression {
            Expression::CallExpression(call) => {
                if let Expression::Identifier(identifier) = &call.callee {
                    if identifier.name == resolve_name {
                        if let Some(argument) = call.arguments.first().and_then(argument_expression)
                        {
                            targets
                                .fulfilled
                                .extend(self.fulfilled_targets_from_expression(argument, env));
                        }
                        return;
                    }
                    if identifier.name == reject_name {
                        if let Some(argument) = call.arguments.first().and_then(argument_expression)
                        {
                            targets
                                .rejected
                                .extend(self.callable_targets_from_expression(argument, env));
                        }
                        return;
                    }
                }
                for argument in &call.arguments {
                    if let Some(argument) = argument_expression(argument) {
                        self.collect_promise_executor_expression(
                            argument,
                            resolve_name,
                            reject_name,
                            env,
                            targets,
                        );
                    }
                }
            }
            Expression::SequenceExpression(sequence) => {
                for expression in &sequence.expressions {
                    self.collect_promise_executor_expression(
                        expression,
                        resolve_name,
                        reject_name,
                        env,
                        targets,
                    );
                }
            }
            Expression::ParenthesizedExpression(expression) => self
                .collect_promise_executor_expression(
                    &expression.expression,
                    resolve_name,
                    reject_name,
                    env,
                    targets,
                ),
            _ => {}
        }
    }

    fn promise_method_source(
        &self,
        call: &'ast CallExpression<'ast>,
        env: &FlowEnv,
    ) -> Option<(PromiseTargets, &'ast str)> {
        let Expression::StaticMemberExpression(member) = &call.callee else {
            return None;
        };
        let method = member.property.name.as_str();
        if !matches!(method, "then" | "catch" | "finally") {
            return None;
        }
        let source = self.promise_targets_from_expression(&member.object, env)?;
        Some((source, method))
    }

    fn collect_promise_handler_flows(
        &mut self,
        call: &'ast CallExpression<'ast>,
        method: &str,
        source: &PromiseTargets,
        env: &FlowEnv,
    ) {
        match method {
            "then" => {
                self.collect_promise_handler_argument(call, 0, source.fulfilled.clone(), env);
                self.collect_promise_handler_argument(call, 1, source.rejected.clone(), env);
            }
            "catch" => {
                self.collect_promise_handler_argument(call, 0, source.rejected.clone(), env);
            }
            "finally" => {}
            _ => {}
        }
    }

    fn collect_promise_handler_argument(
        &mut self,
        call: &'ast CallExpression<'ast>,
        argument_index: usize,
        targets: CollectionTargets,
        env: &FlowEnv,
    ) {
        if targets.is_empty() {
            return;
        }
        let Some(expression) = call
            .arguments
            .get(argument_index)
            .and_then(argument_expression)
        else {
            return;
        };
        let Some(callback) = self.function_for_expression(expression) else {
            return;
        };
        self.collect_callback_value_flows(callback, expression, vec![(0, targets)], env);
    }

    fn collect_callback_value_flows(
        &mut self,
        callback: &'db FunctionFact,
        expression: &'ast Expression<'ast>,
        bindings: Vec<(usize, CollectionTargets)>,
        parent_env: &FlowEnv,
    ) {
        let patterns = callback_param_patterns(expression);
        let mut env = parent_env.clone();
        for (index, targets) in bindings {
            if let Some(pattern) = patterns.get(index) {
                bind_param_pattern(pattern, &targets, &mut env);
            }
        }
        // Calls inside this nested body belong to `callback`, not to any
        // `caller_override` active in the enclosing constructor/field body (e.g.
        // super5's `super.m()` inside an IIFE arrow is attributed to the arrow,
        // not the class node). `current_super` is intentionally preserved: an
        // arrow lexically captures `super` from its enclosing method.
        let saved_override = self.caller_override.take();
        match expression {
            Expression::ArrowFunctionExpression(function) => {
                if let Some(expression) = function.get_expression() {
                    self.collect_expression(expression, callback, &mut env);
                } else {
                    for statement in &function.body.statements {
                        self.collect_statement(statement, callback, &mut env);
                    }
                }
            }
            Expression::FunctionExpression(function) => {
                if let Some(body) = function.body.as_deref() {
                    for statement in &body.statements {
                        self.collect_statement(statement, callback, &mut env);
                    }
                }
            }
            _ => {}
        }
        self.caller_override = saved_override;
    }

    fn promise_method_result_targets(
        &self,
        call: &'ast CallExpression<'ast>,
        method: &str,
        source: &PromiseTargets,
        env: &FlowEnv,
    ) -> PromiseTargets {
        match method {
            "then" => {
                let mut result = PromiseTargets::default();
                if let Some(callback) = call.arguments.first().and_then(argument_expression) {
                    result.merge(self.promise_targets_from_callback_return(callback, env));
                } else {
                    result.fulfilled.extend(source.fulfilled.clone());
                }
                if let Some(callback) = call.arguments.get(1).and_then(argument_expression) {
                    result.merge(self.promise_targets_from_callback_return(callback, env));
                } else {
                    result.rejected.extend(source.rejected.clone());
                }
                result
            }
            "catch" => {
                let mut result = PromiseTargets {
                    fulfilled: source.fulfilled.clone(),
                    rejected: CollectionTargets::default(),
                };
                if let Some(callback) = call.arguments.first().and_then(argument_expression) {
                    result.merge(self.promise_targets_from_callback_return(callback, env));
                } else {
                    result.rejected.extend(source.rejected.clone());
                }
                result
            }
            "finally" => source.clone(),
            _ => PromiseTargets::default(),
        }
    }

    fn promise_targets_from_callback_return(
        &self,
        callback: &'ast Expression<'ast>,
        env: &FlowEnv,
    ) -> PromiseTargets {
        let Some(expression) = callback_returned_expression(callback) else {
            return PromiseTargets::default();
        };
        if let Some(promise) = self.promise_targets_from_expression(expression, env) {
            return promise;
        }
        PromiseTargets {
            fulfilled: self.callable_targets_from_expression(expression, env),
            rejected: CollectionTargets::default(),
        }
    }

    fn callable_targets_from_expression(
        &self,
        expression: &'ast Expression<'ast>,
        env: &FlowEnv,
    ) -> CollectionTargets {
        if let Expression::ParenthesizedExpression(expression) = expression {
            return self.callable_targets_from_expression(&expression.expression, env);
        }
        if let Some(function) = self.function_for_expression(expression) {
            return CollectionTargets {
                keys: Vec::new(),
                values: vec![function.id],
                object_values: Vec::new(),
                ..Default::default()
            };
        }
        if let Expression::StaticMemberExpression(member) = expression
            && let Some(object) = self.object_targets_from_expression(&member.object, env)
            && let Some(targets) = object.properties.get(member.property.name.as_str())
        {
            return CollectionTargets {
                keys: Vec::new(),
                values: targets.clone(),
                object_values: Vec::new(),
                ..Default::default()
            };
        }
        if let Expression::ComputedMemberExpression(member) = expression
            && let Some(object) = self.object_targets_from_expression(&member.object, env)
            && let Some(property) = self.constant_string_from_expression(&member.expression, env)
            && let Some(targets) = object.properties.get(&property)
        {
            return CollectionTargets {
                keys: Vec::new(),
                values: targets.clone(),
                object_values: Vec::new(),
                ..Default::default()
            };
        }
        if let Some(collection) = self.collection_targets_from_expression(expression, env) {
            return collection;
        }
        // A bare reference to a top-level `function` declaration used as a value
        // (`{ value: f1 }`, `obj.x = f1`) resolves to that function, unless a
        // local binding of the same name shadows it.
        if let Expression::Identifier(identifier) = expression
            && !env.bindings.contains_key(identifier.name.as_str())
            && let Some(flow) = self.function_declarations.get(identifier.name.as_str())
        {
            return CollectionTargets {
                keys: Vec::new(),
                values: vec![flow.function.id],
                object_values: Vec::new(),
                ..Default::default()
            };
        }
        CollectionTargets::default()
    }

    fn fulfilled_targets_from_expression(
        &self,
        expression: &'ast Expression<'ast>,
        env: &FlowEnv,
    ) -> CollectionTargets {
        if let Some(promise) = self.promise_targets_from_expression(expression, env) {
            return promise.fulfilled;
        }
        self.callable_targets_from_expression(expression, env)
    }

    fn aggregate_promise_targets(
        &self,
        expression: &'ast Expression<'ast>,
        env: &FlowEnv,
    ) -> PromiseTargets {
        let mut targets = PromiseTargets::default();
        let Expression::ArrayExpression(array) = expression else {
            return targets;
        };
        for element in &array.elements {
            if let Some(expression) = array_element_expression(element)
                && let Some(promise) = self.promise_targets_from_expression(expression, env)
            {
                targets.merge(promise);
            }
        }
        targets
    }

    fn all_settled_promise_targets(
        &self,
        expression: &'ast Expression<'ast>,
        env: &FlowEnv,
    ) -> PromiseTargets {
        let mut fulfilled = CollectionTargets::default();
        let Expression::ArrayExpression(array) = expression else {
            return PromiseTargets {
                fulfilled,
                rejected: CollectionTargets::default(),
            };
        };
        for element in &array.elements {
            let Some(expression) = array_element_expression(element) else {
                continue;
            };
            let Some(promise) = self.promise_targets_from_expression(expression, env) else {
                continue;
            };
            let mut result = ObjectTargets::default();
            for target in promise.fulfilled.all_targets() {
                result.add_property_target("value".to_string(), target);
            }
            for target in promise.rejected.all_targets() {
                result.add_property_target("reason".to_string(), target);
            }
            fulfilled.object_values.push(result);
        }
        PromiseTargets {
            fulfilled,
            rejected: CollectionTargets::default(),
        }
    }

    fn array_literal_targets(
        &self,
        array: &'ast oxc_ast::ast::ArrayExpression<'ast>,
        env: Option<&FlowEnv>,
    ) -> CollectionTargets {
        let mut targets = CollectionTargets::default();
        for element in &array.elements {
            let Some(expression) = array_element_expression(element) else {
                continue;
            };
            if let Some(function) = self.function_for_expression(expression) {
                targets.values.push(function.id);
                continue;
            }
            if let Some(env) = env {
                if let Some(object) = self.object_targets_from_expression(expression, env) {
                    targets.object_values.push(object);
                    continue;
                }
                targets.append_ordered(self.callable_targets_from_expression(expression, env));
            }
        }
        targets
    }

    fn array_of_targets(
        &self,
        call: &'ast CallExpression<'ast>,
        env: &FlowEnv,
    ) -> CollectionTargets {
        let mut targets = CollectionTargets::default();
        for argument in call.arguments.iter().filter_map(argument_expression) {
            targets.append_ordered(
                self.targets_from_argument_expression(argument, env)
                    .unwrap_or_default(),
            );
        }
        targets
    }

    fn concat_targets(
        &self,
        source: &CollectionTargets,
        call: &'ast CallExpression<'ast>,
        env: &FlowEnv,
    ) -> CollectionTargets {
        let mut targets = CollectionTargets {
            values: source.values.clone(),
            object_values: source.object_values.clone(),
            unknown: source.unknown.clone(),
            unknown_objects: source.unknown_objects.clone(),
            dynamic: source.dynamic.clone(),
            ..Default::default()
        };
        for argument in call.arguments.iter().filter_map(argument_expression) {
            if let Some(collection) = self.collection_targets_from_expression(argument, env) {
                targets.append_ordered(collection);
            } else {
                targets.append_ordered(
                    self.targets_from_argument_expression(argument, env)
                        .unwrap_or_default(),
                );
            }
        }
        targets
    }

    fn object_literal_targets(
        &self,
        object: &'ast oxc_ast::ast::ObjectExpression<'ast>,
        env: Option<&FlowEnv>,
    ) -> ObjectTargets {
        let mut targets = ObjectTargets::default();
        for property in &object.properties {
            let ObjectPropertyKind::ObjectProperty(property) = property else {
                continue;
            };
            let names = self.property_key_names(&property.key, property.computed, env);
            if names.is_empty() {
                continue;
            };
            if let Some(function) = self.function_for_object_property(property) {
                for name in names {
                    match property.kind {
                        PropertyKind::Get => targets.add_getter_target(name, function.id),
                        PropertyKind::Set => targets.add_setter_target(name, function.id),
                        PropertyKind::Init => targets.add_property_target(name, function.id),
                    }
                }
                continue;
            }
            if let Expression::ObjectExpression(value) = &property.value {
                let nested = self.object_literal_targets(value, env);
                for name in names {
                    if name == "__proto__" {
                        targets.merge(nested.clone());
                    } else {
                        targets.add_object_property(name, nested.clone());
                    }
                }
            } else if let Some(env) = env
                && let Some(collection) =
                    self.collection_targets_from_expression(&property.value, env)
            {
                for name in names {
                    targets.add_collection_property(name, collection.clone());
                }
            } else if matches!(&property.value, Expression::ThisExpression(_))
                && let Some(env) = env
            {
                for name in names {
                    targets.add_object_property(name, env.this_object.clone());
                }
            } else if let Some(env) = env
                && let Expression::Identifier(_) = &property.value
                && let Some(object) = self.object_targets_from_expression(&property.value, env)
            {
                // A property whose value references an object binding
                // (`{ f: descr }` where `const descr = { value: fn }`) inlines
                // that object's shape. `{ __proto__: Base }` instead links the
                // prototype, so the base's members are inherited directly.
                for name in names {
                    if name == "__proto__" {
                        targets.merge(object.clone());
                    } else {
                        targets.add_object_property(name, object.clone());
                    }
                }
            } else if let Some(env) = env {
                // A property whose value is a reference to a callable (a `function`
                // declaration or a bound callable variable), e.g. a descriptor's
                // `{ value: f1 }`, binds the property to that function.
                let callables = self.callable_targets_from_expression(&property.value, env);
                for name in names {
                    for id in &callables.values {
                        targets.add_property_target(name.clone(), *id);
                    }
                }
            }
        }
        targets
    }

    fn map_entries_targets_from_expression(
        &self,
        expression: &'ast Expression<'ast>,
        env: &FlowEnv,
    ) -> CollectionTargets {
        match expression {
            Expression::Identifier(identifier) => env
                .bindings
                .get(identifier.name.as_str())
                .cloned()
                .unwrap_or_default(),
            Expression::ArrayExpression(array) => {
                let mut targets = CollectionTargets::default();
                for element in &array.elements {
                    let Some(Expression::ArrayExpression(entry)) =
                        array_element_expression(element)
                    else {
                        continue;
                    };
                    if let Some(key) = entry
                        .elements
                        .first()
                        .and_then(array_element_expression)
                        .and_then(|expression| self.function_for_expression(expression))
                    {
                        targets.keys.push(key.id);
                    }
                    if let Some(value) = entry
                        .elements
                        .get(1)
                        .and_then(array_element_expression)
                        .and_then(|expression| self.function_for_expression(expression))
                    {
                        targets.values.push(value.id);
                    }
                }
                targets
            }
            Expression::ParenthesizedExpression(expression) => {
                self.map_entries_targets_from_expression(&expression.expression, env)
            }
            _ => CollectionTargets::default(),
        }
    }

    fn array_from_targets(
        &self,
        call: &'ast CallExpression<'ast>,
        env: &FlowEnv,
    ) -> Option<CollectionTargets> {
        let mut targets = call
            .arguments
            .first()
            .and_then(argument_expression)
            .and_then(|argument| self.collection_targets_from_expression(argument, env))?;
        if let Some(callback) = call
            .arguments
            .get(1)
            .and_then(argument_expression)
            .and_then(callback_returned_function)
            .and_then(|expression| self.function_for_expression(expression))
        {
            targets.values = vec![callback.id];
            targets.keys.clear();
        }
        Some(targets)
    }

    fn emit_binding_targets(
        &mut self,
        owner: &'db FunctionFact,
        name: &str,
        span: oxc_span::Span,
        targets: Option<Vec<FunctionId>>,
    ) {
        let Some(targets) = targets else {
            return;
        };
        if targets.is_empty() {
            return;
        }
        for site in self.sites.iter().copied().filter(|site| {
            !site.in_throw
                && site.caller == owner.id
                && site.span.start_byte >= span.start
                && site.span.end_byte <= span.end
                && matches!(
                    &site.callee,
                    CallCallee::Identifier { name: callee, .. } if callee == name
                )
        }) {
            for target in &targets {
                self.rows.push(TsCallableFlow {
                    site: site.id,
                    caller: self.caller_override.unwrap_or(site.caller),
                    target_function: *target,
                    kind: self.value_flow_kind(),
                    binding: name.to_owned(),
                });
            }
        }
    }

    fn emit_member_targets(
        &mut self,
        owner: &'db FunctionFact,
        property: &str,
        span: oxc_span::Span,
        targets: Option<Vec<FunctionId>>,
    ) {
        let Some(targets) = targets else {
            return;
        };
        if targets.is_empty() {
            return;
        }
        for site in self.sites.iter().copied().filter(|site| {
            !site.in_throw
                && site.caller == owner.id
                && site.span.start_byte >= span.start
                && site.span.end_byte <= span.end
                && matches!(
                    &site.callee,
                    CallCallee::Member {
                        property: callee,
                        ..
                    } if callee == property
                )
        }) {
            for target in &targets {
                self.rows.push(TsCallableFlow {
                    site: site.id,
                    caller: self.caller_override.unwrap_or(site.caller),
                    target_function: *target,
                    kind: self.value_flow_kind(),
                    binding: property.to_owned(),
                });
            }
        }
    }

    fn emit_call_span_targets(
        &mut self,
        owner: &'db FunctionFact,
        span: oxc_span::Span,
        binding: &str,
        targets: Option<Vec<FunctionId>>,
    ) {
        let Some(targets) = targets else {
            return;
        };
        if targets.is_empty() {
            return;
        }
        for site in self.sites.iter().copied().filter(|site| {
            !site.in_throw
                && site.caller == owner.id
                && site.span.start_byte == span.start
                && site.span.end_byte == span.end
        }) {
            for target in &targets {
                self.rows.push(TsCallableFlow {
                    site: site.id,
                    caller: self.caller_override.unwrap_or(site.caller),
                    target_function: *target,
                    kind: self.value_flow_kind(),
                    binding: binding.to_owned(),
                });
            }
        }
    }

    fn emit_containing_call_span_targets(
        &mut self,
        owner: &'db FunctionFact,
        span: oxc_span::Span,
        binding: &str,
        targets: Option<Vec<FunctionId>>,
    ) {
        let Some(targets) = targets else {
            return;
        };
        if targets.is_empty() {
            return;
        }
        let Some(site) = self
            .sites
            .iter()
            .copied()
            .filter(|site| {
                !site.in_throw
                    && site.caller == owner.id
                    && site.span.start_byte <= span.start
                    && site.span.end_byte >= span.end
            })
            .min_by_key(|site| site.span.end_byte - site.span.start_byte)
        else {
            return;
        };
        for target in &targets {
            self.rows.push(TsCallableFlow {
                site: site.id,
                caller: self.caller_override.unwrap_or(site.caller),
                target_function: *target,
                kind: self.value_flow_kind(),
                binding: binding.to_owned(),
            });
        }
    }

    fn function_for_expression(
        &self,
        expression: &'ast Expression<'ast>,
    ) -> Option<&'db FunctionFact> {
        // A call or member expression can start at the same byte as its callee.
        // That shared start does not make the expression's result a function.
        match expression {
            Expression::FunctionExpression(function) => self.function_for_span(function.span),
            Expression::ArrowFunctionExpression(function) => {
                self.function_for_span(function.span)
            }
            Expression::ClassExpression(class) => self.function_for_span(class.span),
            Expression::ParenthesizedExpression(inner) => {
                self.function_for_expression(&inner.expression)
            }
            _ => None,
        }
    }

    /// The `FunctionFact` that owns a class member's body. A class *is* its
    /// constructor callable, so a `constructor(){}` body is owned by the class
    /// function (the class span); every other member owns its own body. This
    /// must agree with the MIR body owner, or `site.caller == owner.id` below
    /// rejects every call site in the body.
    fn owner_for_class_method(
        &self,
        class: &'ast Class<'ast>,
        method: &'ast MethodDefinition<'ast>,
    ) -> Option<&'db FunctionFact> {
        if method.kind == MethodDefinitionKind::Constructor {
            return self.function_for_span(class.span);
        }
        self.function_for_class_method(method)
    }

    fn function_for_class_method(
        &self,
        method: &'ast MethodDefinition<'ast>,
    ) -> Option<&'db FunctionFact> {
        let span = class_method_function_span(method);
        self.function_for_span(span)
            .or_else(|| self.function_for_span(method.span))
            .or_else(|| self.function_inside_span(method.span))
    }

    fn function_for_object_property(
        &self,
        property: &'ast ObjectProperty<'ast>,
    ) -> Option<&'db FunctionFact> {
        let span = object_property_function_span(property);
        self.function_for_span(span)
            .or_else(|| self.function_for_expression(&property.value))
    }

    fn function_for_span(&self, span: oxc_span::Span) -> Option<&'db FunctionFact> {
        let functions = self.functions_by_start.get(&(self.file.id, span.start))?;
        functions
            .iter()
            .copied()
            .find(|function| function.span.end_byte == span.end)
            .or_else(|| functions.first().copied())
    }

    fn function_inside_span(&self, span: oxc_span::Span) -> Option<&'db FunctionFact> {
        self.functions_by_start
            .range((self.file.id, span.start)..=(self.file.id, span.end))
            .flat_map(|(_, functions)| functions.iter().copied())
            .filter(|function| function.span.end_byte <= span.end)
            .min_by_key(|function| {
                (
                    function.span.end_byte - function.span.start_byte,
                    function.id.0,
                )
            })
    }
}

#[derive(Clone, Debug, Default)]
struct FlowEnv {
    bindings: BTreeMap<String, CollectionTargets>,
    bound_functions: BTreeMap<String, Vec<BoundFunctionTarget>>,
    objects: BTreeMap<String, ObjectTargets>,
    /// Variable name -> class key in `classes`, for classes flowed through a
    /// variable (e.g. `var a = makeClass()` then `new a()` / `a.staticM()`).
    class_bindings: BTreeMap<String, String>,
    string_bindings: BTreeMap<String, String>,
    bool_bindings: BTreeMap<String, bool>,
    string_arrays: BTreeMap<String, Vec<String>>,
    promises: BTreeMap<String, PromiseTargets>,
    async_functions: BTreeMap<String, PromiseTargets>,
    async_generators: BTreeMap<String, CollectionTargets>,
    async_iterators: BTreeMap<String, CollectionTargets>,
    /// Ordered yield/return sequence of a generator **function** bound to a name
    /// (`function* g(){…}` / `const g = async function*(){…}`), used to resolve
    /// the k-th `.next()` on an instance of it to the k-th yielded value.
    generator_sequences: BTreeMap<String, GeneratorSequence>,
    /// Ordered yield/return sequence of an **iterator instance** bound to a name
    /// (`const it = g()`), so `it.next()` is sequenced positionally and `for-of`
    /// over `it` iterates the yields without the `return` value.
    iterator_sequences: BTreeMap<String, GeneratorSequence>,
    this_object: ObjectTargets,
    this_callables: CollectionTargets,
    /// Loop variables bound by a `for (const k in obj)` head. A computed access
    /// `obj[k]` keyed on such a variable ranges over every own property, so it
    /// resolves to the union of the object's property values.
    forin_keys: std::collections::BTreeSet<String>,
    /// Variable names bound to a `merge-descriptors`-style property-copy helper
    /// (`var mixin = require('merge-descriptors')`). A call `mixin(dst, src, …)`
    /// copies every own property of each source onto `dst` in place (like the
    /// named-target `Object.assign`), which is how Express assembles `app`.
    merge_helpers: std::collections::BTreeSet<String>,
    /// Explicit prototype object of a named object, recorded when
    /// `Object.setPrototypeOf(o, p)` / `o.__proto__ = p` is seen. Used to bind
    /// `super` while walking `o`'s own methods (object-literal `super.m()`).
    /// Kept separate from the prototype's members being merged into `o` (which
    /// drives direct `o.m()` dispatch) so `super` resolves to the proto only.
    object_prototypes: BTreeMap<String, ObjectTargets>,
    /// Parameter names that are KNOWN to be `undefined` in this invocation because
    /// the resolved call passed fewer positional arguments than the function
    /// declares (a syntactic arg-count shortfall — never inferred from value
    /// analysis). Lets `decide_branch` fold a guard like `if (depth == null)` to its
    /// live arm, so the dead arm's calls are never walked (demand-driven pruning of
    /// `array-flatten`'s `flattenWithDepth` recursion under express's no-`depth` call).
    absent: std::collections::BTreeSet<String>,
}

/// The ordered result sequence of a generator: each `yield` (with `yield*`
/// flattened to one slot per delegated value) is a step, and the `return` value
/// is delivered only by the terminal `.next()` (and excluded from `for-of`).
/// Jelly models generators sequence-sensitively — the first `.next()` yields the
/// first value, the next `.next()` the second, etc. — so resolving every
/// `.next()` to every yield (the flow-insensitive lump) over-approximates.
#[derive(Clone, Debug, Default)]
struct GeneratorSequence {
    steps: Vec<CollectionTargets>,
    ret: CollectionTargets,
}

impl GeneratorSequence {
    /// The result delivered by the `ordinal`-th `.next()` (0-based): the matching
    /// yield, or the `return` value at the terminal step (one past the yields).
    fn result_at(&self, ordinal: usize) -> CollectionTargets {
        match ordinal.cmp(&self.steps.len()) {
            std::cmp::Ordering::Less => self.steps[ordinal].clone(),
            std::cmp::Ordering::Equal => self.ret.clone(),
            std::cmp::Ordering::Greater => CollectionTargets::default(),
        }
    }

    /// The values produced by iterating (`for-of` / `for-await`) from `offset`
    /// onward — the remaining yields, never the `return` value.
    fn iterated(&self, offset: usize) -> CollectionTargets {
        let mut targets = CollectionTargets::default();
        for step in self.steps.iter().skip(offset) {
            targets.extend(step.clone());
        }
        targets
    }

    /// Positionally union another sequence's steps (and return value) into this
    /// one, for a `.next()` whose receiver could be one of several generators.
    fn merge(&mut self, other: GeneratorSequence) {
        for (index, step) in other.steps.into_iter().enumerate() {
            if index < self.steps.len() {
                self.steps[index].extend(step);
            } else {
                self.steps.push(step);
            }
        }
        self.ret.extend(other.ret);
    }
}

#[derive(Clone, Debug)]
struct BoundFunctionTarget {
    function: FunctionId,
    this_object: ObjectTargets,
    this_callables: CollectionTargets,
    bound_arguments: Vec<CollectionTargets>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct CollectionTargets {
    keys: Vec<FunctionId>,
    /// Index-ordered array element cells (numeric indices). `value_at(i)` reads
    /// `values[i]`, so the insertion order MUST be preserved in array contexts
    /// (use [`append_ordered`], not the sorting [`extend`]) for specific-index and
    /// out-of-bounds reads to stay precise. Mirrors the points-to heap's
    /// numeric-index cells.
    values: Vec<FunctionId>,
    object_values: Vec<ObjectTargets>,
    /// The LIFO-appended slice of Jelly's `%ARRAY_UNKNOWN`: `push`/`unshift`/spread
    /// elements. Read by `pop`/`shift`/`find` (which dynamically returns an appended
    /// element) and unioned into [`all_targets`]/[`elements`] (so `for…of` /
    /// `forEach` callbacks still see them), but NEVER by `value_at` (a specific-index
    /// read stays precise — unioning the unknown cell into index reads
    /// over-approximates the single runtime element and adds FPs fast).
    unknown: Vec<FunctionId>,
    unknown_objects: Vec<ObjectTargets>,
    /// Callables written at an arbitrary / dynamic index (`arr[i] = fn`,
    /// `arr[k] = fn`). Like `unknown` these have no specific index, so they feed
    /// iteration ([`all_targets`]/[`elements`], for `for…of`/`forEach` recall) but
    /// are NOT returned by `pop`/`shift` — the dynamic oracle pops the LIFO-pushed
    /// element, not an element placed at some other index (`arrays.js` `x[42]=`,
    /// `x[x]=` must not surface from `x.pop()`).
    dynamic: Vec<FunctionId>,
}

impl CollectionTargets {
    fn extend(&mut self, other: Self) {
        self.keys.extend(other.keys);
        self.values.extend(other.values);
        self.object_values.extend(other.object_values);
        self.unknown.extend(other.unknown);
        self.unknown_objects.extend(other.unknown_objects);
        self.dynamic.extend(other.dynamic);
        self.keys.sort();
        self.keys.dedup();
        self.values.sort();
        self.values.dedup();
        self.unknown.sort();
        self.unknown.dedup();
        self.dynamic.sort();
        self.dynamic.dedup();
    }

    fn append_ordered(&mut self, other: Self) {
        for key in other.keys {
            if !self.keys.contains(&key) {
                self.keys.push(key);
            }
        }
        for value in other.values {
            if !self.values.contains(&value) {
                self.values.push(value);
            }
        }
        self.object_values.extend(other.object_values);
        for value in other.unknown {
            if !self.unknown.contains(&value) {
                self.unknown.push(value);
            }
        }
        self.unknown_objects.extend(other.unknown_objects);
        for value in other.dynamic {
            if !self.dynamic.contains(&value) {
                self.dynamic.push(value);
            }
        }
    }

    /// Record callables appended at the array's end (`push`/`unshift`/spread) into
    /// the pop-readable `%ARRAY_UNKNOWN` bucket.
    fn push_unknown(&mut self, targets: Vec<FunctionId>) {
        self.unknown.extend(targets);
        self.unknown.sort();
        self.unknown.dedup();
    }

    /// Record callables written at an arbitrary / dynamic index (`arr[i] = fn`).
    /// These feed iteration but not `pop`/`shift` (see the `dynamic` field).
    fn push_dynamic(&mut self, targets: Vec<FunctionId>) {
        self.dynamic.extend(targets);
        self.dynamic.sort();
        self.dynamic.dedup();
    }

    fn is_empty(&self) -> bool {
        self.keys.is_empty()
            && self.values.is_empty()
            && self.object_values.is_empty()
            && self.unknown.is_empty()
            && self.unknown_objects.is_empty()
            && self.dynamic.is_empty()
    }

    fn all_targets(&self) -> Vec<FunctionId> {
        let mut targets = self.values.clone();
        targets.extend(self.keys.iter().copied());
        targets.extend(self.unknown.iter().copied());
        targets.extend(self.dynamic.iter().copied());
        targets.sort();
        targets.dedup();
        targets
    }

    /// The array's element callables: numeric-index cells plus the appended
    /// (`%ARRAY_UNKNOWN`) and dynamic-index-write buckets, but NOT map keys. Used to
    /// bind `for…of` / `forEach`/`map`/`reduce` callback element parameters, which
    /// genuinely visit every element including the dynamically-stored ones.
    fn elements(&self) -> Vec<FunctionId> {
        let mut targets = self.values.clone();
        targets.extend(self.unknown.iter().copied());
        targets.extend(self.dynamic.iter().copied());
        targets.sort();
        targets.dedup();
        targets
    }

    fn keys_or_values(&self) -> Vec<FunctionId> {
        if self.keys.is_empty() {
            self.values.clone()
        } else {
            self.keys.clone()
        }
    }

    fn value_at(&self, index: usize) -> Self {
        Self {
            values: self.values.get(index).copied().into_iter().collect(),
            object_values: self.object_values.get(index).cloned().into_iter().collect(),
            ..Default::default()
        }
    }

    fn values_from(&self, index: usize) -> Self {
        Self {
            values: self.values.iter().skip(index).copied().collect(),
            object_values: self.object_values.iter().skip(index).cloned().collect(),
            ..Default::default()
        }
    }

    fn object_at(&self, index: usize) -> Option<ObjectTargets> {
        self.object_values.get(index).cloned()
    }

    fn singular_object(&self) -> Option<ObjectTargets> {
        (self.object_values.len() == 1).then(|| self.object_values[0].clone())
    }

    fn argument_list(&self) -> Vec<Self> {
        let len = self.values.len().max(self.object_values.len());
        let mut args: Vec<Self> = (0..len).map(|index| self.value_at(index)).collect();
        // Appended (`push`/spread) and dynamic-index-write elements have no specific
        // index, so when an array is spread as call arguments (`f.apply(t, arr)`)
        // append them as one further positional argument — otherwise a
        // `const a = []; a.push(f); bar.apply(t, a)` spread would bind no argument.
        let mut spread = self.unknown.clone();
        spread.extend(self.dynamic.iter().copied());
        spread.sort();
        spread.dedup();
        if !spread.is_empty() || !self.unknown_objects.is_empty() {
            args.push(Self {
                values: spread,
                object_values: self.unknown_objects.clone(),
                ..Default::default()
            });
        }
        args
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ObjectTargets {
    properties: BTreeMap<String, Vec<FunctionId>>,
    getter_properties: BTreeMap<String, Vec<FunctionId>>,
    setter_properties: BTreeMap<String, Vec<FunctionId>>,
    object_properties: BTreeMap<String, Box<ObjectTargets>>,
    collection_properties: BTreeMap<String, CollectionTargets>,
    /// Callable values reachable through a **dynamic** (computed, non-constant)
    /// property write keyed on a loop/iteration variable, e.g.
    /// `methods.forEach(m => { app[m] = fn })`. A read of a property with no
    /// explicit entry falls back to this lane (the write side of the for-in
    /// computed-read dual). Express attaches its HTTP-verb methods this way, so
    /// `app.get`/`app.post`/… all resolve to the one verb function here.
    wildcard: Vec<FunctionId>,
}

impl ObjectTargets {
    fn is_empty(&self) -> bool {
        self.properties.is_empty()
            && self.getter_properties.is_empty()
            && self.setter_properties.is_empty()
            && self.object_properties.is_empty()
            && self.collection_properties.is_empty()
            && self.wildcard.is_empty()
    }

    fn add_wildcard_target(&mut self, target: FunctionId) {
        self.wildcard.push(target);
        self.wildcard.sort();
        self.wildcard.dedup();
    }

    /// Resolve a property read for a method call: the explicit entry if present,
    /// otherwise the dynamic `wildcard` lane (a property attached through a
    /// computed write over an iteration variable). Explicit entries do **not**
    /// union the wildcard — only genuinely-absent names fall through — so a
    /// statically-defined method (`app.listen`) is not polluted by the verb
    /// wildcard.
    fn resolve_call_property(&self, name: &str) -> Option<Vec<FunctionId>> {
        if let Some(targets) = self.properties.get(name) {
            return Some(targets.clone());
        }
        (!self.wildcard.is_empty()).then(|| self.wildcard.clone())
    }

    fn add_property_target(&mut self, name: String, target: FunctionId) {
        let targets = self.properties.entry(name).or_default();
        targets.push(target);
        targets.sort();
        targets.dedup();
    }

    fn add_getter_target(&mut self, name: String, target: FunctionId) {
        let targets = self.getter_properties.entry(name).or_default();
        targets.push(target);
        targets.sort();
        targets.dedup();
    }

    fn add_setter_target(&mut self, name: String, target: FunctionId) {
        let targets = self.setter_properties.entry(name).or_default();
        targets.push(target);
        targets.sort();
        targets.dedup();
    }

    fn add_object_property(&mut self, name: String, targets: ObjectTargets) {
        self.object_properties.insert(name, Box::new(targets));
    }

    fn add_collection_property(&mut self, name: String, targets: CollectionTargets) {
        self.collection_properties.insert(name, targets);
    }

    fn merge(&mut self, other: Self) {
        for (name, mut targets) in other.properties {
            let slot = self.properties.entry(name).or_default();
            slot.append(&mut targets);
            slot.sort();
            slot.dedup();
        }
        for (name, mut targets) in other.getter_properties {
            let slot = self.getter_properties.entry(name).or_default();
            slot.append(&mut targets);
            slot.sort();
            slot.dedup();
        }
        for (name, mut targets) in other.setter_properties {
            let slot = self.setter_properties.entry(name).or_default();
            slot.append(&mut targets);
            slot.sort();
            slot.dedup();
        }
        for (name, targets) in other.object_properties {
            self.object_properties.entry(name).or_insert(targets);
        }
        for (name, targets) in other.collection_properties {
            self.collection_properties.entry(name).or_insert(targets);
        }
        self.wildcard.extend(other.wildcard);
        self.wildcard.sort();
        self.wildcard.dedup();
    }

    /// Merge `other` on top of `self`, with `other`'s names **replacing** (not
    /// unioning) `self`'s. Models prototype-chain shadowing: a subclass member
    /// overrides the inherited member of the same name (`x.m()` resolves to the
    /// child's `m`, not both child's and parent's). Shadowing crosses kinds — a
    /// child data property shadows an inherited accessor of the same name and vice
    /// versa — so the inherited name is dropped from every kind before merging.
    fn override_with(&mut self, other: Self) {
        let shadowed: std::collections::BTreeSet<&str> = other
            .properties
            .keys()
            .chain(other.getter_properties.keys())
            .chain(other.setter_properties.keys())
            .chain(other.object_properties.keys())
            .chain(other.collection_properties.keys())
            .map(String::as_str)
            .collect();
        let shadowed: Vec<String> = shadowed.into_iter().map(ToOwned::to_owned).collect();
        for name in &shadowed {
            self.properties.remove(name);
            self.getter_properties.remove(name);
            self.setter_properties.remove(name);
            self.object_properties.remove(name);
            self.collection_properties.remove(name);
        }
        // `self` now has no entry for any name `other` defines, so the union in
        // `merge` is equivalent to insert for those names while leaving inherited
        // names untouched.
        self.merge(other);
    }

    fn without_properties(&self, used: &[String]) -> Self {
        let used = used.iter().collect::<std::collections::BTreeSet<_>>();
        Self {
            properties: self
                .properties
                .iter()
                .filter(|(name, _)| !used.contains(name))
                .map(|(name, targets)| (name.clone(), targets.clone()))
                .collect(),
            getter_properties: self
                .getter_properties
                .iter()
                .filter(|(name, _)| !used.contains(name))
                .map(|(name, targets)| (name.clone(), targets.clone()))
                .collect(),
            setter_properties: self
                .setter_properties
                .iter()
                .filter(|(name, _)| !used.contains(name))
                .map(|(name, targets)| (name.clone(), targets.clone()))
                .collect(),
            object_properties: self
                .object_properties
                .iter()
                .filter(|(name, _)| !used.contains(name))
                .map(|(name, targets)| (name.clone(), targets.clone()))
                .collect(),
            collection_properties: self
                .collection_properties
                .iter()
                .filter(|(name, _)| !used.contains(name))
                .map(|(name, targets)| (name.clone(), targets.clone()))
                .collect(),
            wildcard: self.wildcard.clone(),
        }
    }
}

fn add_assignment_to_object_property(
    object: &mut ObjectTargets,
    property: String,
    callable_targets: &[FunctionId],
    nested_object: Option<ObjectTargets>,
    nested_collection: Option<CollectionTargets>,
) {
    for target in callable_targets {
        object.add_property_target(property.clone(), *target);
    }
    if let Some(nested_object) = nested_object {
        object.add_object_property(property.clone(), nested_object);
    }
    if let Some(nested_collection) = nested_collection {
        object.add_collection_property(property, nested_collection);
    }
}

fn descriptor_for_property(source: &ObjectTargets, property: &str) -> ObjectTargets {
    let mut descriptor = ObjectTargets::default();
    if let Some(targets) = source.properties.get(property) {
        for target in targets {
            descriptor.add_property_target("value".to_string(), *target);
        }
    }
    if let Some(targets) = source.object_properties.get(property) {
        descriptor.add_object_property("value".to_string(), (**targets).clone());
    }
    if let Some(targets) = source.collection_properties.get(property) {
        descriptor.add_collection_property("value".to_string(), targets.clone());
    }
    descriptor
}

/// The full property-descriptor map of an object (`Object.getOwnPropertyDescriptors`):
/// each property becomes `{ value }` (or `{ get }` / `{ set }` for accessors), so
/// feeding the result back through `defineProperties`/`create` round-trips.
fn descriptors_for_object(source: &ObjectTargets) -> ObjectTargets {
    let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    names.extend(source.properties.keys().cloned());
    names.extend(source.object_properties.keys().cloned());
    names.extend(source.collection_properties.keys().cloned());
    names.extend(source.getter_properties.keys().cloned());
    names.extend(source.setter_properties.keys().cloned());

    let mut descriptors = ObjectTargets::default();
    for name in names {
        let mut descriptor = descriptor_for_property(source, &name);
        if let Some(getters) = source.getter_properties.get(&name) {
            for getter in getters {
                descriptor.add_property_target("get".to_string(), *getter);
            }
        }
        if let Some(setters) = source.setter_properties.get(&name) {
            for setter in setters {
                descriptor.add_property_target("set".to_string(), *setter);
            }
        }
        descriptors.add_object_property(name, descriptor);
    }
    descriptors
}

fn copy_descriptor_value_to_property(
    target: &mut ObjectTargets,
    property: String,
    descriptor: &ObjectTargets,
) {
    if let Some(targets) = descriptor.properties.get("value") {
        for target_id in targets {
            target.add_property_target(property.clone(), *target_id);
        }
    }
    if let Some(targets) = descriptor.object_properties.get("value") {
        target.add_object_property(property.clone(), (**targets).clone());
    }
    if let Some(targets) = descriptor.collection_properties.get("value") {
        target.add_collection_property(property, targets.clone());
    }
}

/// Apply a single property descriptor (`{ value }`, `{ get }`, or `{ set }`) to
/// `target[property]`, mapping the descriptor's accessor functions to the
/// object's getter/setter slots so `obj.prop` / `obj.prop = x` resolve.
fn copy_descriptor_to_property(
    target: &mut ObjectTargets,
    property: String,
    descriptor: &ObjectTargets,
) {
    copy_descriptor_value_to_property(target, property.clone(), descriptor);
    if let Some(getters) = descriptor.properties.get("get") {
        for getter in getters {
            target.add_getter_target(property.clone(), *getter);
        }
    }
    if let Some(setters) = descriptor.properties.get("set") {
        for setter in setters {
            target.add_setter_target(property.clone(), *setter);
        }
    }
}

/// Apply a property-descriptor map (`{ f: { value }, foo: { get } }`, the second
/// argument of `Object.defineProperties` / `Object.create`) to `target`.
fn apply_descriptor_map(target: &mut ObjectTargets, descriptors: &ObjectTargets) {
    for (name, descriptor) in &descriptors.object_properties {
        copy_descriptor_to_property(target, name.clone(), descriptor);
    }
}

#[derive(Clone, Debug, Default)]
struct ClassTargets {
    instance_object: ObjectTargets,
    static_object: ObjectTargets,
    constructor_assignments: Vec<ConstructorAssignment>,
    instance_self_aliases: Vec<String>,
    static_self_aliases: Vec<String>,
    super_name: Option<String>,
    /// The class node (constructor-callable) `FunctionFact`, used to emit the
    /// `new v()` -> constructor edge when the class is flowed through a variable.
    constructor: Option<FunctionId>,
}

impl ClassTargets {
    fn is_empty(&self) -> bool {
        self.instance_object.is_empty()
            && self.static_object.is_empty()
            && self.constructor_assignments.is_empty()
            && self.instance_self_aliases.is_empty()
            && self.static_self_aliases.is_empty()
            && self.super_name.is_none()
    }
}

#[derive(Clone, Debug)]
enum ConstructorAssignment {
    Param { property: String, index: usize },
}

#[derive(Clone, Debug, Default)]
struct PromiseTargets {
    fulfilled: CollectionTargets,
    rejected: CollectionTargets,
}

impl PromiseTargets {
    fn merge(&mut self, other: Self) {
        self.fulfilled.extend(other.fulfilled);
        self.rejected.extend(other.rejected);
    }
}

fn sites_by_file(sites: &[CallSiteFact]) -> BTreeMap<FileId, Vec<&CallSiteFact>> {
    let mut by_file = BTreeMap::<FileId, Vec<&CallSiteFact>>::new();
    for site in sites {
        by_file.entry(site.file).or_default().push(site);
    }
    by_file
}

fn functions_by_file_start(db: &impl AnalysisHost) -> BTreeMap<(FileId, u32), Vec<&FunctionFact>> {
    let mut by_start = BTreeMap::<(FileId, u32), Vec<&FunctionFact>>::new();
    for function in db
        .functions()
        .iter()
        .filter(|function| function.language.is_ts_family())
    {
        by_start
            .entry((function.file, function.span.start_byte))
            .or_default()
            .push(function);
    }
    for functions in by_start.values_mut() {
        functions.sort_by_key(|function| (function.span.end_byte, function.id.0));
    }
    by_start
}

fn module_function<'db>(
    db: &'db impl AnalysisHost,
    file: &SourceFile,
) -> Option<&'db FunctionFact> {
    db.functions().iter().find(|function| {
        function.file == file.id
            && function.language == file.language
            && function.name == TS_JS_MODULE_FUNCTION_NAME
    })
}

fn binding_identifier_name(pattern: &BindingPattern<'_>) -> Option<String> {
    match pattern {
        BindingPattern::BindingIdentifier(identifier) => Some(identifier.name.to_string()),
        BindingPattern::AssignmentPattern(pattern) => binding_identifier_name(&pattern.left),
        _ => None,
    }
}

fn module_export_name_string(name: &ModuleExportName<'_>) -> String {
    match name {
        ModuleExportName::IdentifierName(identifier) => identifier.name.to_string(),
        ModuleExportName::IdentifierReference(identifier) => identifier.name.to_string(),
        ModuleExportName::StringLiteral(literal) => literal.value.to_string(),
    }
}

/// Which CommonJS export slot an assignment target writes to.
enum ExportAssignmentTarget {
    /// `module.exports = ...`
    WholeExports,
    /// `exports.foo = ...` or `module.exports.foo = ...`
    Property(String),
}

/// Does this expression evaluate to the module's export object? Recognizes
/// `exports`, `module.exports`, and assignment chains that write to one of them
/// (`exports = module.exports = {}`), so `var app = exports = module.exports = {}`
/// registers `app` as an export alias.
fn expression_aliases_exports(expression: &Expression<'_>) -> bool {
    match expression {
        Expression::Identifier(identifier) => identifier.name.as_str() == "exports",
        Expression::StaticMemberExpression(member) => {
            expression_identifier(&member.object) == Some("module")
                && member.property.name.as_str() == "exports"
        }
        Expression::ParenthesizedExpression(inner) => expression_aliases_exports(&inner.expression),
        Expression::AssignmentExpression(assignment) => {
            assignment_target_is_exports(&assignment.left)
                || expression_aliases_exports(&assignment.right)
        }
        _ => false,
    }
}

/// Is this assignment target `exports` or `module.exports` (the whole export
/// object, not a property of it)?
fn assignment_target_is_exports(target: &AssignmentTarget<'_>) -> bool {
    match target {
        AssignmentTarget::AssignmentTargetIdentifier(identifier) => {
            identifier.name.as_str() == "exports"
        }
        _ => matches!(
            export_assignment_target(target),
            Some(ExportAssignmentTarget::WholeExports)
        ),
    }
}

fn export_assignment_target(target: &AssignmentTarget<'_>) -> Option<ExportAssignmentTarget> {
    let AssignmentTarget::StaticMemberExpression(member) = target else {
        return None;
    };
    let property = member.property.name.as_str();
    match expression_identifier(&member.object) {
        // module.exports = ...
        Some("module") if property == "exports" => {
            return Some(ExportAssignmentTarget::WholeExports);
        }
        Some("module") => return None,
        // exports.foo = ...
        Some("exports") => {
            return Some(ExportAssignmentTarget::Property(property.to_string()));
        }
        _ => {}
    }
    // module.exports.foo = ...
    if let Expression::StaticMemberExpression(inner) = &member.object
        && expression_identifier(&inner.object) == Some("module")
        && inner.property.name.as_str() == "exports"
    {
        return Some(ExportAssignmentTarget::Property(property.to_string()));
    }
    None
}

fn bind_collection_pattern(
    pattern: &BindingPattern<'_>,
    targets: &CollectionTargets,
    env: &mut FlowEnv,
) {
    match pattern {
        BindingPattern::BindingIdentifier(identifier) => {
            if let Some(object) = targets.singular_object() {
                env.objects.insert(identifier.name.to_string(), object);
            }
            env.bindings
                .insert(identifier.name.to_string(), targets.clone());
        }
        BindingPattern::ArrayPattern(array) => {
            for (index, element) in array.elements.iter().enumerate() {
                if let Some(element) = element {
                    bind_collection_pattern(element, &targets.value_at(index), env);
                }
            }
            if let Some(rest) = &array.rest {
                bind_collection_pattern(
                    &rest.argument,
                    &targets.values_from(array.elements.len()),
                    env,
                );
            }
        }
        BindingPattern::AssignmentPattern(pattern) => {
            bind_collection_pattern(&pattern.left, targets, env);
        }
        _ => {}
    }
}

/// Whether control definitely leaves the enclosing block at `statement`
/// (`return`/`throw`, or a block whose last statement does), so statements after
/// it are unreachable. Used to prune the tail after an absence-folded early-return
/// guard (`if (depth == null) { return … }`).
fn branch_terminates(statement: &Statement<'_>) -> bool {
    match statement {
        Statement::ReturnStatement(_) | Statement::ThrowStatement(_) => true,
        Statement::BlockStatement(block) => block.body.last().is_some_and(branch_terminates),
        _ => false,
    }
}

fn param_pattern_from_binding(pattern: &BindingPattern<'_>) -> ParamPattern {
    match pattern {
        BindingPattern::BindingIdentifier(identifier) => {
            ParamPattern::Binding(identifier.name.to_string())
        }
        BindingPattern::ArrayPattern(array) => ParamPattern::Array {
            elements: array
                .elements
                .iter()
                .map(|element| element.as_ref().map(param_pattern_from_binding))
                .collect(),
            rest: array
                .rest
                .as_ref()
                .map(|rest| Box::new(param_pattern_from_binding(&rest.argument))),
        },
        BindingPattern::ObjectPattern(object) => ParamPattern::Object {
            properties: object
                .properties
                .iter()
                .filter_map(|property| {
                    static_property_key(&property.key, property.computed)
                        .map(|name| (name, param_pattern_from_binding(&property.value)))
                })
                .collect(),
            rest: object
                .rest
                .as_ref()
                .map(|rest| Box::new(param_pattern_from_binding(&rest.argument))),
        },
        BindingPattern::AssignmentPattern(pattern) => param_pattern_from_binding(&pattern.left),
    }
}

fn bind_param_pattern(pattern: &ParamPattern, targets: &CollectionTargets, env: &mut FlowEnv) {
    match pattern {
        ParamPattern::Binding(name) => {
            if let Some(object) = targets.singular_object() {
                env.objects.insert(name.clone(), object);
            }
            env.bindings.insert(name.clone(), targets.clone());
        }
        ParamPattern::Array { elements, rest } => {
            for (index, element) in elements.iter().enumerate() {
                if let Some(element) = element {
                    bind_param_pattern(element, &targets.value_at(index), env);
                }
            }
            if let Some(rest) = rest {
                bind_param_pattern(rest, &targets.values_from(elements.len()), env);
            }
        }
        ParamPattern::Object { .. } => {}
    }
}

fn bind_param_object_pattern(pattern: &ParamPattern, targets: &ObjectTargets, env: &mut FlowEnv) {
    match pattern {
        ParamPattern::Binding(name) => {
            env.objects.insert(name.clone(), targets.clone());
        }
        ParamPattern::Object { properties, rest } => {
            let mut used = Vec::new();
            for (name, pattern) in properties {
                used.push(name.clone());
                if let Some(values) = targets.properties.get(name) {
                    bind_param_pattern(
                        pattern,
                        &CollectionTargets {
                            keys: Vec::new(),
                            values: values.clone(),
                            object_values: Vec::new(),
                            ..Default::default()
                        },
                        env,
                    );
                }
            }
            if let Some(rest) = rest {
                bind_param_object_pattern(rest, &targets.without_properties(&used), env);
            }
        }
        ParamPattern::Array { .. } => {}
    }
}

fn collection_assignment_target_name<'a>(target: &'a AssignmentTarget<'a>) -> Option<&'a str> {
    match target {
        AssignmentTarget::ComputedMemberExpression(member) => expression_identifier(&member.object),
        AssignmentTarget::StaticMemberExpression(member) => expression_identifier(&member.object),
        _ => None,
    }
}

/// Property name(s) of a `this.X = …` (static) or `this["f" + "oo"] = …`
/// (computed literal/concatenation key) assignment, resolved **without** an env
/// (pure string literals and `+` concatenation only) for use in the structural
/// constructor pre-scan. `this[name]` with a variable key resolves nothing here.
fn this_assignment_static_or_concat_keys(target: &AssignmentTarget<'_>) -> Vec<String> {
    match target {
        AssignmentTarget::StaticMemberExpression(member)
            if matches!(&member.object, Expression::ThisExpression(_)) =>
        {
            vec![member.property.name.to_string()]
        }
        AssignmentTarget::ComputedMemberExpression(member)
            if matches!(&member.object, Expression::ThisExpression(_)) =>
        {
            const_string_without_env(&member.expression)
                .into_iter()
                .collect()
        }
        _ => Vec::new(),
    }
}

/// Resolve a pure string-literal / `+`-concatenation expression to its value,
/// with no environment (`"a"`, `"f" + "oo"`, `("p" + "1")`). Returns `None` for
/// anything needing a binding lookup.
fn const_string_without_env(expression: &Expression<'_>) -> Option<String> {
    match expression {
        Expression::StringLiteral(literal) => Some(literal.value.to_string()),
        Expression::ParenthesizedExpression(inner) => const_string_without_env(&inner.expression),
        Expression::BinaryExpression(binary) if binary.operator == BinaryOperator::Addition => {
            let left = const_string_without_env(&binary.left)?;
            let right = const_string_without_env(&binary.right)?;
            Some(left + &right)
        }
        _ => None,
    }
}

fn this_assignment_property(target: &AssignmentTarget<'_>) -> Option<String> {
    match target {
        AssignmentTarget::StaticMemberExpression(member)
            if matches!(&member.object, Expression::ThisExpression(_)) =>
        {
            Some(member.property.name.to_string())
        }
        _ => None,
    }
}

/// BDD/TDD test-runner globals (Mocha/Jasmine/Jest) that invoke their function
/// argument. Used to walk those callbacks as host-invoked (reachable) bodies.
fn is_test_framework_global(name: &str) -> bool {
    matches!(
        name,
        "describe"
            | "it"
            | "before"
            | "after"
            | "beforeEach"
            | "afterEach"
            | "beforeAll"
            | "afterAll"
            | "context"
            | "specify"
            | "suite"
            | "test"
            | "suiteSetup"
            | "suiteTeardown"
            | "setup"
            | "teardown"
            | "xdescribe"
            | "xit"
            | "fdescribe"
            | "fit"
    )
}

fn prototype_member_assignment_name<'a>(
    target: &'a AssignmentTarget<'a>,
) -> Option<(&'a str, &'a str)> {
    let AssignmentTarget::StaticMemberExpression(member) = target else {
        return None;
    };
    let Expression::StaticMemberExpression(object) = &member.object else {
        return None;
    };
    if object.property.name != "prototype" {
        return None;
    }
    let constructor = expression_identifier(&object.object)?;
    Some((constructor, member.property.name.as_str()))
}

/// The object name in `obj.__proto__ = …` (a dynamic prototype link).
/// The expression a parenthesized/comma-sequence evaluates to: unwrap parens and
/// take the last operand of a sequence (`(a(), fn)` → `fn`). Used to resolve a
/// class field initialized to such an expression to its trailing function value.
fn field_value_expression<'a>(value: &'a Expression<'a>) -> &'a Expression<'a> {
    match value {
        Expression::ParenthesizedExpression(paren) => field_value_expression(&paren.expression),
        Expression::SequenceExpression(sequence) => sequence
            .expressions
            .last()
            .map_or(value, field_value_expression),
        _ => value,
    }
}

/// The first `super(...)` call among a constructor's top-level statements.
fn find_super_call<'a>(statements: &'a [Statement<'a>]) -> Option<&'a CallExpression<'a>> {
    statements.iter().find_map(|statement| {
        let Statement::ExpressionStatement(expression) = statement else {
            return None;
        };
        let Expression::CallExpression(call) = &expression.expression else {
            return None;
        };
        matches!(&call.callee, Expression::Super(_)).then(|| call.as_ref())
    })
}

/// The `constructor` method of a class, if it declares one explicitly.
fn constructor_method<'a>(class: &'a Class<'a>) -> Option<&'a MethodDefinition<'a>> {
    class.body.body.iter().find_map(|element| match element {
        ClassElement::MethodDefinition(method)
            if method.kind == MethodDefinitionKind::Constructor =>
        {
            Some(method.as_ref())
        }
        _ => None,
    })
}

/// The property name in `super.X` (a static member access on `super`), used to
/// resolve a `static g = super.X` field alias against the superclass's members.
fn super_member_property<'a>(value: &'a Expression<'a>) -> Option<&'a str> {
    let Expression::StaticMemberExpression(member) = value else {
        return None;
    };
    matches!(&member.object, Expression::Super(_)).then(|| member.property.name.as_str())
}

fn proto_assignment_object_name<'a>(target: &'a AssignmentTarget<'a>) -> Option<&'a str> {
    let AssignmentTarget::StaticMemberExpression(member) = target else {
        return None;
    };
    if member.property.name != "__proto__" {
        return None;
    }
    expression_identifier(&member.object)
}

/// The constructor name in `C.prototype = …` (assigning a whole object to the
/// prototype, as opposed to `C.prototype.m = …` or `C.prototype = new S()`).
fn prototype_object_assignment_name<'a>(target: &'a AssignmentTarget<'a>) -> Option<&'a str> {
    let AssignmentTarget::StaticMemberExpression(member) = target else {
        return None;
    };
    if member.property.name != "prototype" {
        return None;
    }
    expression_identifier(&member.object)
}

fn prototype_assignment_super_name<'a>(
    target: &'a AssignmentTarget<'a>,
    value: &'a Expression<'a>,
) -> Option<(&'a str, &'a str)> {
    let AssignmentTarget::StaticMemberExpression(member) = target else {
        return None;
    };
    if member.property.name != "prototype" {
        return None;
    }
    let constructor = expression_identifier(&member.object)?;
    let Expression::NewExpression(new_expression) = value else {
        return None;
    };
    let super_name = callee_identifier(&new_expression.callee)?;
    Some((constructor, super_name))
}

fn param_index_for_expression(
    expression: &Expression<'_>,
    params: &[ParamPattern],
) -> Option<usize> {
    let name = expression_identifier(expression)?;
    params
        .iter()
        .enumerate()
        .find_map(|(index, param)| match param {
            ParamPattern::Binding(param_name) if param_name == name => Some(index),
            _ => None,
        })
}

fn callback_argument_index(method: &str) -> Option<usize> {
    match method {
        "forEach" | "map" | "filter" | "find" | "findIndex" | "some" | "every" | "sort"
        | "flatMap" | "reduce" | "reduceRight" => Some(0),
        "from" => Some(1),
        _ => None,
    }
}

fn callback_targets_for_parameter(
    method: &str,
    index: usize,
    collection: &CollectionTargets,
) -> Vec<FunctionId> {
    match method {
        "forEach" if index == 0 => collection.elements(),
        "forEach" if index == 1 => {
            // `Map.forEach` is `(value, key, map)`: the 2nd parameter is the key,
            // so bind it to the collection's keys. Arrays (2nd param is a numeric
            // index) and Sets (`(value, value, set)`) have no distinct keys, so
            // fall back to the elements for them.
            if collection.keys.is_empty() {
                collection.elements()
            } else {
                collection.keys.clone()
            }
        }
        "reduce" | "reduceRight" if index == 1 => collection.elements(),
        _ if index == 0 => collection.elements(),
        _ => Vec::new(),
    }
}

fn iterator_result_object(values: &CollectionTargets) -> ObjectTargets {
    let mut result = ObjectTargets::default();
    for target in values.all_targets() {
        result.add_property_target("value".to_string(), target);
    }
    result
}

fn callback_param_patterns(expression: &Expression<'_>) -> Vec<ParamPattern> {
    match expression {
        Expression::ArrowFunctionExpression(function) => function
            .params
            .items
            .iter()
            .map(|param| param_pattern_from_binding(&param.pattern))
            .collect(),
        Expression::FunctionExpression(function) => function
            .params
            .items
            .iter()
            .map(|param| param_pattern_from_binding(&param.pattern))
            .collect(),
        Expression::ParenthesizedExpression(expression) => {
            callback_param_patterns(&expression.expression)
        }
        _ => Vec::new(),
    }
}

fn callback_returned_expression<'a>(expression: &'a Expression<'a>) -> Option<&'a Expression<'a>> {
    match expression {
        Expression::ArrowFunctionExpression(function) => function
            .get_expression()
            .or_else(|| returned_expression_from_statements(&function.body.statements)),
        Expression::FunctionExpression(function) => function
            .body
            .as_deref()
            .and_then(|body| returned_expression_from_statements(&body.statements)),
        Expression::ParenthesizedExpression(expression) => {
            callback_returned_expression(&expression.expression)
        }
        _ => None,
    }
}

fn async_arrow_returned_expression<'a>(
    function: &'a oxc_ast::ast::ArrowFunctionExpression<'a>,
) -> Option<&'a Expression<'a>> {
    function
        .get_expression()
        .or_else(|| returned_expression_from_statements(&function.body.statements))
}

fn returned_expression_from_statements<'a>(
    statements: &'a oxc_allocator::Vec<'a, Statement<'a>>,
) -> Option<&'a Expression<'a>> {
    statements
        .iter()
        .find_map(returned_expression_from_statement)
}

fn returned_expression_from_statement<'a>(
    statement: &'a Statement<'a>,
) -> Option<&'a Expression<'a>> {
    match statement {
        Statement::ReturnStatement(statement) => statement.argument.as_ref(),
        Statement::BlockStatement(block) => returned_expression_from_statements(&block.body),
        Statement::IfStatement(statement) => {
            returned_expression_from_statement(&statement.consequent).or_else(|| {
                statement
                    .alternate
                    .as_ref()
                    .and_then(|alternate| returned_expression_from_statement(alternate))
            })
        }
        _ => None,
    }
}

fn callback_returned_function<'a>(expression: &'a Expression<'a>) -> Option<&'a Expression<'a>> {
    match expression {
        Expression::ArrowFunctionExpression(function) => function
            .get_expression()
            .and_then(expression_as_function)
            .or_else(|| returned_function_from_statements(&function.body.statements)),
        Expression::FunctionExpression(function) => function
            .body
            .as_deref()
            .and_then(|body| returned_function_from_statements(&body.statements)),
        Expression::ParenthesizedExpression(expression) => {
            callback_returned_function(&expression.expression)
        }
        _ => None,
    }
}

fn returned_function_from_statements<'a>(
    statements: &'a oxc_allocator::Vec<'a, Statement<'a>>,
) -> Option<&'a Expression<'a>> {
    statements.iter().find_map(returned_function_from_statement)
}

fn returned_function_from_statement<'a>(
    statement: &'a Statement<'a>,
) -> Option<&'a Expression<'a>> {
    match statement {
        Statement::ReturnStatement(statement) => {
            statement.argument.as_ref().and_then(expression_as_function)
        }
        Statement::BlockStatement(block) => returned_function_from_statements(&block.body),
        Statement::IfStatement(statement) => {
            returned_function_from_statement(&statement.consequent).or_else(|| {
                statement
                    .alternate
                    .as_ref()
                    .and_then(|alternate| returned_function_from_statement(alternate))
            })
        }
        _ => None,
    }
}

fn expression_as_function<'a>(expression: &'a Expression<'a>) -> Option<&'a Expression<'a>> {
    match expression {
        Expression::ArrowFunctionExpression(_) | Expression::FunctionExpression(_) => {
            Some(expression)
        }
        Expression::ParenthesizedExpression(expression) => {
            expression_as_function(&expression.expression)
        }
        _ => None,
    }
}

fn call_expression_callee<'a>(expression: &'a Expression<'a>) -> Option<&'a CallExpression<'a>> {
    match expression {
        Expression::CallExpression(call) => Some(call),
        Expression::ParenthesizedExpression(expression) => {
            call_expression_callee(&expression.expression)
        }
        _ => None,
    }
}

fn callback_parameter_names(expression: &Expression<'_>) -> Vec<String> {
    match expression {
        Expression::ArrowFunctionExpression(function) => function
            .params
            .items
            .iter()
            .filter_map(|param| binding_identifier_name(&param.pattern))
            .collect(),
        Expression::FunctionExpression(function) => function
            .params
            .items
            .iter()
            .filter_map(|param| binding_identifier_name(&param.pattern))
            .collect(),
        Expression::ParenthesizedExpression(expression) => {
            callback_parameter_names(&expression.expression)
        }
        _ => Vec::new(),
    }
}

fn argument_expression<'a>(argument: &'a Argument<'a>) -> Option<&'a Expression<'a>> {
    match argument {
        Argument::SpreadElement(spread) => Some(&spread.argument),
        _ => Some(argument.to_expression()),
    }
}

fn array_element_expression<'a>(
    element: &'a ArrayExpressionElement<'a>,
) -> Option<&'a Expression<'a>> {
    match element {
        ArrayExpressionElement::SpreadElement(spread) => Some(&spread.argument),
        ArrayExpressionElement::Elision(_) => None,
        _ => Some(element.to_expression()),
    }
}

fn expression_identifier<'a>(expression: &'a Expression<'a>) -> Option<&'a str> {
    match expression {
        Expression::Identifier(identifier) => Some(identifier.name.as_str()),
        Expression::ParenthesizedExpression(expression) => {
            expression_identifier(&expression.expression)
        }
        _ => None,
    }
}

/// Span-derived synthetic key for a class expression. Unique per source location,
/// so it never collides with a class declaration's name in the `classes` map.
fn class_expression_key(class: &Class<'_>) -> String {
    format!("@class@{}:{}", class.span.start, class.span.end)
}

fn numeric_index(expression: &Expression<'_>) -> Option<usize> {
    match expression {
        Expression::NumericLiteral(literal)
            if literal.value.is_finite() && literal.value.fract() == 0.0 =>
        {
            usize::try_from(literal.value as u64).ok()
        }
        Expression::ParenthesizedExpression(expression) => numeric_index(&expression.expression),
        _ => None,
    }
}

fn callee_identifier<'a>(expression: &'a Expression<'a>) -> Option<&'a str> {
    match expression {
        Expression::Identifier(identifier) => Some(identifier.name.as_str()),
        Expression::ParenthesizedExpression(expression) => {
            callee_identifier(&expression.expression)
        }
        _ => None,
    }
}

fn static_member_call<'a>(expression: &'a Expression<'a>) -> Option<(&'a str, &'a str)> {
    match expression {
        Expression::StaticMemberExpression(member) => {
            let object = expression_identifier(&member.object)?;
            Some((object, member.property.name.as_str()))
        }
        Expression::ParenthesizedExpression(expression) => {
            static_member_call(&expression.expression)
        }
        _ => None,
    }
}

/// Record every `receiver.next()` call's `span.start` under its receiver name,
/// recursing through nested statements and function bodies. Traversal order is
/// irrelevant — the caller sorts by span to derive ordinals.
fn index_next_calls_in_statement(statement: &Statement<'_>, out: &mut BTreeMap<String, Vec<u32>>) {
    match statement {
        Statement::ExpressionStatement(statement) => {
            index_next_calls_in_expression(&statement.expression, out);
        }
        Statement::VariableDeclaration(variable) => {
            for declarator in &variable.declarations {
                if let Some(init) = &declarator.init {
                    index_next_calls_in_expression(init, out);
                }
            }
        }
        Statement::ReturnStatement(statement) => {
            if let Some(argument) = &statement.argument {
                index_next_calls_in_expression(argument, out);
            }
        }
        Statement::IfStatement(statement) => {
            index_next_calls_in_expression(&statement.test, out);
            index_next_calls_in_statement(&statement.consequent, out);
            if let Some(alternate) = &statement.alternate {
                index_next_calls_in_statement(alternate, out);
            }
        }
        Statement::BlockStatement(block) => {
            for statement in &block.body {
                index_next_calls_in_statement(statement, out);
            }
        }
        Statement::ForOfStatement(statement) => {
            index_next_calls_in_expression(&statement.right, out);
            index_next_calls_in_statement(&statement.body, out);
        }
        Statement::ForInStatement(statement) => {
            index_next_calls_in_expression(&statement.right, out);
            index_next_calls_in_statement(&statement.body, out);
        }
        Statement::ForStatement(statement) => {
            if let Some(test) = &statement.test {
                index_next_calls_in_expression(test, out);
            }
            if let Some(update) = &statement.update {
                index_next_calls_in_expression(update, out);
            }
            index_next_calls_in_statement(&statement.body, out);
        }
        Statement::WhileStatement(statement) => {
            index_next_calls_in_expression(&statement.test, out);
            index_next_calls_in_statement(&statement.body, out);
        }
        Statement::DoWhileStatement(statement) => {
            index_next_calls_in_statement(&statement.body, out);
            index_next_calls_in_expression(&statement.test, out);
        }
        Statement::TryStatement(statement) => {
            for inner in &statement.block.body {
                index_next_calls_in_statement(inner, out);
            }
            if let Some(handler) = &statement.handler {
                for inner in &handler.body.body {
                    index_next_calls_in_statement(inner, out);
                }
            }
            if let Some(finalizer) = &statement.finalizer {
                for inner in &finalizer.body {
                    index_next_calls_in_statement(inner, out);
                }
            }
        }
        Statement::SwitchStatement(statement) => {
            index_next_calls_in_expression(&statement.discriminant, out);
            for case in &statement.cases {
                if let Some(test) = &case.test {
                    index_next_calls_in_expression(test, out);
                }
                for inner in &case.consequent {
                    index_next_calls_in_statement(inner, out);
                }
            }
        }
        Statement::FunctionDeclaration(function) => {
            if let Some(body) = &function.body {
                for inner in &body.statements {
                    index_next_calls_in_statement(inner, out);
                }
            }
        }
        Statement::ClassDeclaration(class) => {
            index_next_calls_in_class(class, out);
        }
        _ => {}
    }
}

fn index_next_calls_in_class(class: &Class<'_>, out: &mut BTreeMap<String, Vec<u32>>) {
    for element in &class.body.body {
        match element {
            ClassElement::MethodDefinition(method) => {
                if let Some(body) = method.value.body.as_deref() {
                    for inner in &body.statements {
                        index_next_calls_in_statement(inner, out);
                    }
                }
            }
            ClassElement::StaticBlock(block) => {
                for inner in &block.body {
                    index_next_calls_in_statement(inner, out);
                }
            }
            ClassElement::PropertyDefinition(property) => {
                if let Some(value) = &property.value {
                    index_next_calls_in_expression(value, out);
                }
            }
            _ => {}
        }
    }
}

fn index_next_calls_in_expression(
    expression: &Expression<'_>,
    out: &mut BTreeMap<String, Vec<u32>>,
) {
    match expression {
        Expression::CallExpression(call) => {
            if let Some((name, "next")) = static_member_call(&call.callee) {
                out.entry(name.to_string())
                    .or_default()
                    .push(call.span.start);
            }
            index_next_calls_in_expression(&call.callee, out);
            for argument in &call.arguments {
                if let Some(inner) = argument_expression(argument) {
                    index_next_calls_in_expression(inner, out);
                }
            }
        }
        Expression::NewExpression(new) => {
            index_next_calls_in_expression(&new.callee, out);
            for argument in &new.arguments {
                if let Some(inner) = argument_expression(argument) {
                    index_next_calls_in_expression(inner, out);
                }
            }
        }
        Expression::StaticMemberExpression(member) => {
            index_next_calls_in_expression(&member.object, out);
        }
        Expression::ComputedMemberExpression(member) => {
            index_next_calls_in_expression(&member.object, out);
            index_next_calls_in_expression(&member.expression, out);
        }
        Expression::PrivateFieldExpression(member) => {
            index_next_calls_in_expression(&member.object, out);
        }
        Expression::ParenthesizedExpression(inner) => {
            index_next_calls_in_expression(&inner.expression, out);
        }
        Expression::AwaitExpression(inner) => {
            index_next_calls_in_expression(&inner.argument, out);
        }
        Expression::UnaryExpression(inner) => {
            index_next_calls_in_expression(&inner.argument, out);
        }
        Expression::YieldExpression(inner) => {
            if let Some(argument) = &inner.argument {
                index_next_calls_in_expression(argument, out);
            }
        }
        Expression::SequenceExpression(sequence) => {
            for inner in &sequence.expressions {
                index_next_calls_in_expression(inner, out);
            }
        }
        Expression::AssignmentExpression(assignment) => {
            index_next_calls_in_expression(&assignment.right, out);
        }
        Expression::ConditionalExpression(conditional) => {
            index_next_calls_in_expression(&conditional.test, out);
            index_next_calls_in_expression(&conditional.consequent, out);
            index_next_calls_in_expression(&conditional.alternate, out);
        }
        Expression::LogicalExpression(logical) => {
            index_next_calls_in_expression(&logical.left, out);
            index_next_calls_in_expression(&logical.right, out);
        }
        Expression::BinaryExpression(binary) => {
            index_next_calls_in_expression(&binary.left, out);
            index_next_calls_in_expression(&binary.right, out);
        }
        Expression::ArrowFunctionExpression(function) => {
            for inner in &function.body.statements {
                index_next_calls_in_statement(inner, out);
            }
        }
        Expression::FunctionExpression(function) => {
            if let Some(body) = &function.body {
                for inner in &body.statements {
                    index_next_calls_in_statement(inner, out);
                }
            }
        }
        Expression::ObjectExpression(object) => {
            for property in &object.properties {
                match property {
                    ObjectPropertyKind::ObjectProperty(property) => {
                        index_next_calls_in_expression(&property.value, out);
                    }
                    ObjectPropertyKind::SpreadProperty(spread) => {
                        index_next_calls_in_expression(&spread.argument, out);
                    }
                }
            }
        }
        Expression::ArrayExpression(array) => {
            for element in &array.elements {
                if let Some(inner) = array_element_expression(element) {
                    index_next_calls_in_expression(inner, out);
                }
            }
        }
        Expression::TemplateLiteral(template) => {
            for inner in &template.expressions {
                index_next_calls_in_expression(inner, out);
            }
        }
        _ => {}
    }
}

fn static_property_key(key: &PropertyKey<'_>, computed: bool) -> Option<String> {
    if computed {
        return match key {
            PropertyKey::StringLiteral(literal) => Some(literal.value.to_string()),
            PropertyKey::NumericLiteral(literal) => Some(literal.value.to_string()),
            _ => None,
        };
    }
    match key {
        PropertyKey::StaticIdentifier(identifier) => Some(identifier.name.to_string()),
        PropertyKey::PrivateIdentifier(identifier) => Some(format!("#{}", identifier.name)),
        PropertyKey::StringLiteral(literal) => Some(literal.value.to_string()),
        PropertyKey::NumericLiteral(literal) => Some(literal.value.to_string()),
        _ => None,
    }
}

fn bounded_string_product(left: &[String], right: &[String]) -> Vec<String> {
    let mut values = Vec::new();
    for left in left {
        for right in right {
            values.push(format!("{left}{right}"));
            if values.len() >= 8 {
                sort_dedup_strings(&mut values);
                values.truncate(8);
                return values;
            }
        }
    }
    sort_dedup_strings(&mut values);
    values
}

fn sort_dedup_strings(values: &mut Vec<String>) {
    values.sort();
    values.dedup();
}

fn class_method_function_span(method: &MethodDefinition<'_>) -> oxc_span::Span {
    if method.r#static && method.kind == MethodDefinitionKind::Method {
        return oxc_span::Span::new(method.key.span().start, method.span.end);
    }
    method.span
}

fn object_property_function_span(property: &ObjectProperty<'_>) -> oxc_span::Span {
    if property.method {
        return property.span;
    }
    property.value.span()
}

fn callee_text(expression: &Expression<'_>) -> Option<String> {
    match expression {
        Expression::Identifier(identifier) => Some(identifier.name.to_string()),
        Expression::StaticMemberExpression(member) => {
            callee_text(&member.object).map(|object| format!("{object}.{}", member.property.name))
        }
        Expression::ParenthesizedExpression(expression) => callee_text(&expression.expression),
        _ => None,
    }
}

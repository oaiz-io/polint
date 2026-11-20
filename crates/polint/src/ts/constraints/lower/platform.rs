//! Instantiates validated platform operations as value-flow rows.

use super::*;
use crate::ts::constraints::platform::{Invocation, Method, Operation, embedded};

#[derive(Clone)]
pub(super) struct BoundValue {
    target: FlowVar,
    receiver: FlowVar,
    prefix: Vec<FlowVar>,
}

impl Lowerer<'_> {
    fn global_path(&mut self, expression: &Expression<'_>) -> Option<String> {
        match expression.get_inner_expression() {
            Expression::Identifier(reference) if self.reference_local(reference).is_none() => {
                Some(reference.name.to_string())
            }
            Expression::StaticMemberExpression(member) => Some(format!(
                "{}.{}",
                self.global_path(&member.object)?,
                member.property.name
            )),
            _ => None,
        }
    }

    pub(super) fn platform_call(&mut self, call: &CallExpression<'_>) -> Option<FlowVar> {
        let global = self
            .global_path(&call.callee)
            .map(|path| format!("global:{path}"));
        let member = match call.callee.get_inner_expression() {
            Expression::StaticMemberExpression(member) => {
                Some((&member.object, member.property.name.as_str().to_string()))
            }
            Expression::ComputedMemberExpression(member) => {
                Some((&member.object, self.constant_string(&member.expression, 0)?))
            }
            _ => None,
        };
        let selector = global
            .as_deref()
            .and_then(|name| {
                embedded()?
                    .methods
                    .iter()
                    .find(|method| method.names.iter().any(|n| n == name))
            })
            .or_else(|| {
                let (_, name) = member.as_ref()?;
                let selector = format!("member:{name}");
                embedded()?
                    .methods
                    .iter()
                    .find(|method| method.names.contains(&selector))
            })?;
        let member_name = member.as_ref().map(|(_, name)| name.clone());
        let (receiver, length) = if let Some((object, name)) = member {
            let receiver = self.expression(object);
            // A known own implementation takes precedence over a platform method.
            if self
                .known_fields(receiver)
                .contains_key(&format!("exact:{name}").into_boxed_str())
            {
                return Some(self.ordinary_member_call(call, receiver, &name));
            }
            (receiver, static_length(object))
        } else {
            (FlowVar::Prim, None)
        };
        let (arguments, spread) = self.arguments_of(&call.arguments);
        let key = call
            .arguments
            .first()
            .and_then(Argument::as_expression)
            .and_then(|expression| self.constant_string(expression, 0));
        let property = call
            .arguments
            .get(1)
            .and_then(Argument::as_expression)
            .and_then(|expression| self.constant_string(expression, 0));
        let site = flow_span(normalized_call_expression_span(self.source, call));
        let value = self.instantiate(
            selector,
            site,
            receiver,
            &arguments,
            length,
            key.as_deref(),
            property.as_deref(),
        );
        if spread.is_some() {
            self.sink(value);
        }
        let expression_at =
            |index: usize| call.arguments.get(index).and_then(Argument::as_expression);
        let complete_shape = match selector.operation {
            Some(Operation::Assign) => call
                .arguments
                .iter()
                .skip(1)
                .all(|arg| arg.as_expression().is_some_and(literal_data_object)),
            Some(Operation::DefineProperty) => expression_at(2).is_some_and(literal_data_object),
            Some(Operation::DefineProperties) => {
                expression_at(1).is_some_and(literal_descriptor_map)
            }
            Some(Operation::Create) => expression_at(1).is_none_or(literal_descriptor_map),
            _ => true,
        };
        if !complete_shape {
            self.sink(value);
        }
        if global.is_none()
            && let Some(name) = member_name
        {
            let callee = self.load(receiver, format!("exact:{name}").into());
            let ordinary = self.temp();
            self.flows.calls.push(FlowCall {
                site,
                callee,
                receiver: Some(receiver),
                arguments,
                spread_from: spread,
                result: ordinary,
                constructs: None,
                construct: false,
                model: None,
            });
            return Some(self.join(value, ordinary));
        }
        Some(value)
    }

    fn ordinary_member_call(
        &mut self,
        call: &CallExpression<'_>,
        receiver: FlowVar,
        name: &str,
    ) -> FlowVar {
        let callee = self.load(receiver, format!("exact:{name}").into());
        let (arguments, spread_from) = self.arguments_of(&call.arguments);
        let result = self.temp();
        self.flows.calls.push(FlowCall {
            site: flow_span(normalized_call_expression_span(self.source, call)),
            callee,
            receiver: Some(receiver),
            arguments,
            spread_from,
            result,
            constructs: None,
            construct: false,
            model: None,
        });
        result
    }

    pub(super) fn platform_construct(&mut self, new: &NewExpression<'_>) -> Option<FlowVar> {
        let name = format!("new:{}", self.global_path(&new.callee)?);
        let method = embedded()?
            .methods
            .iter()
            .find(|method| method.names.contains(&name))?;
        let (arguments, _) = self.arguments_of(&new.arguments);
        Some(self.instantiate(
            method,
            flow_span(normalized_new_expression_span(self.source, new)),
            FlowVar::Prim,
            &arguments,
            None,
            None,
            None,
        ))
    }

    fn model_value(
        &mut self,
        path: &str,
        receiver: FlowVar,
        arguments: &[FlowVar],
        result: FlowVar,
        called: &[FlowVar],
    ) -> FlowVar {
        let mut parts = path.split('.');
        let root = parts.next().unwrap_or_default();
        let mut value = match root {
            "$this" => receiver,
            "$result" => result,
            "$prim" => FlowVar::Prim,
            "$open" => FlowVar::Open,
            _ => root
                .strip_prefix("$param")
                .and_then(|p| p.parse::<usize>().ok())
                .map(|p| arguments.get(p).copied().unwrap_or(FlowVar::Prim))
                .or_else(|| {
                    root.strip_prefix("$call")
                        .and_then(|p| p.parse::<usize>().ok())
                        .and_then(|p| called.get(p).copied())
                })
                .unwrap_or(FlowVar::Open),
        };
        for field in parts {
            value = self.load(
                value,
                if field.starts_with('$') {
                    field.into()
                } else {
                    format!("exact:{field}").into()
                },
            );
        }
        value
    }

    fn model_invoke(
        &mut self,
        site: FlowSpan,
        model: &str,
        callee: FlowVar,
        receiver: FlowVar,
        arguments: Vec<FlowVar>,
        spread_from: Option<usize>,
    ) -> FlowVar {
        let result = self.temp();
        // A possible platform call does not close a callback's signature:
        // outside code or an override can pass further values at this boundary.
        let receiver = self.join(receiver, FlowVar::Open);
        let arguments = arguments
            .into_iter()
            .map(|value| self.join(value, FlowVar::Open))
            .collect();
        self.flows.calls.push(FlowCall {
            site,
            callee,
            receiver: Some(receiver),
            arguments,
            spread_from,
            result,
            constructs: None,
            construct: false,
            model: Some(model.into()),
        });
        result
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the invocation reads the enclosing model bindings"
    )]
    fn callback(
        &mut self,
        call: &Invocation,
        site: FlowSpan,
        model: &str,
        receiver: FlowVar,
        arguments: &[FlowVar],
        result: FlowVar,
        called: &[FlowVar],
    ) -> FlowVar {
        let callee = arguments.get(call.param).copied().unwrap_or(FlowVar::Prim);
        let mut args: Vec<_> = call
            .args
            .iter()
            .map(|path| self.model_value(path, receiver, arguments, result, called))
            .collect();
        if let Some(position) = call.args_from {
            args.extend(arguments.iter().skip(position).copied());
        }
        let this = call.this.as_ref().map_or(FlowVar::Prim, |path| {
            self.model_value(path, receiver, arguments, result, called)
        });
        self.model_invoke(site, model, callee, this, args, None)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "a model instance carries its syntax and value bindings"
    )]
    fn instantiate(
        &mut self,
        method: &Method,
        site: FlowSpan,
        receiver: FlowVar,
        arguments: &[FlowVar],
        length: Option<usize>,
        event: Option<&str>,
        property: Option<&str>,
    ) -> FlowVar {
        let result = self.temp();
        let model = method.names[0].as_str();
        if method.allocate.is_some() {
            self.flows.rows.push(FlowRow::Alloc {
                dst: result,
                object: FlowObject::Site(site),
            });
        }
        let arg = |position: usize| arguments.get(position).copied().unwrap_or(FlowVar::Prim);
        let mut called = Vec::new();
        if matches!(method.operation, Some(Operation::Reduce)) {
            match (length, arguments.len() >= 2) {
                (Some(0), false) => return FlowVar::Prim,
                (Some(0), true) => return arg(1),
                (Some(1), false) => return self.load(receiver, "$elements".into()),
                _ => {}
            }
        }
        for call in &method.calls {
            let value = self.callback(call, site, model, receiver, arguments, result, &called);
            called.push(value);
        }
        match method.operation {
            Some(Operation::Bind) => {
                self.copy(result, receiver);
                self.bound_values.insert(
                    result,
                    BoundValue {
                        target: receiver,
                        receiver: arg(0),
                        prefix: arguments.iter().skip(1).copied().collect(),
                    },
                );
            }
            Some(Operation::ForwardCall | Operation::ForwardApply) => {
                let mut values = arguments.iter().skip(1).copied().collect::<Vec<_>>();
                let mut spread = None;
                if matches!(method.operation, Some(Operation::ForwardApply)) {
                    values = self.array_arguments(arg(1)).unwrap_or_else(|| {
                        spread = Some(0);
                        vec![self.load(arg(1), "$elements".into())]
                    });
                }
                let returned = self.model_invoke(site, model, receiver, arg(0), values, spread);
                self.copy(result, returned);
            }
            Some(Operation::Reduce) => {
                let accumulator = called.first().copied().unwrap_or(FlowVar::Open);
                // An unknown cardinality can bypass the callback entirely.
                if length.is_none() {
                    self.copy(accumulator, arg(1));
                    let elements = self.load(receiver, "$elements".into());
                    self.copy(accumulator, elements);
                }
                if let Some(call) = self.flows.calls.last_mut()
                    && let Some(first) = call.arguments.first_mut()
                {
                    *first = accumulator;
                }
            }
            Some(Operation::Assign) => {
                for source in arguments.iter().skip(1) {
                    self.copy_fields(arg(0), *source);
                }
            }
            Some(Operation::DefineProperty) => {
                if let Some(property) = property {
                    self.define_descriptor(
                        site,
                        model,
                        arg(0),
                        &format!("exact:{property}"),
                        arg(2),
                    );
                } else {
                    self.sink(arg(0));
                    self.sink(arg(2));
                }
            }
            Some(Operation::DefineProperties | Operation::Create) => {
                let target = if matches!(method.operation, Some(Operation::Create)) {
                    result
                } else {
                    arg(0)
                };
                let descriptors = self.known_fields(arg(1));
                if descriptors.is_empty() && arg(1) != FlowVar::Prim {
                    self.sink(target);
                }
                for (field, values) in descriptors {
                    for value in values {
                        self.define_descriptor(site, model, target, &field, value);
                    }
                }
            }
            Some(Operation::Values | Operation::Entries) => {
                for (field, values) in self.known_fields(arg(0)) {
                    if !field.starts_with("exact:") {
                        continue;
                    }
                    for value in values {
                        if matches!(method.operation, Some(Operation::Entries)) {
                            let pair = self.temp();
                            self.flows.rows.push(FlowRow::Alloc {
                                dst: pair,
                                object: FlowObject::Site((site.0, site.0)),
                            });
                            self.store(pair, "exact:0".into(), FlowVar::Prim);
                            self.store(pair, "exact:1".into(), value);
                            self.store(pair, "$elements".into(), value);
                            self.store(result, "$elements".into(), pair);
                        } else {
                            self.store(result, "$elements".into(), value);
                        }
                    }
                }
                // Outside values can contribute further properties.
                self.store(result, "$elements".into(), FlowVar::Open);
            }
            Some(Operation::ArrayFrom) => {
                let elements = self.load(arg(0), "$elements".into());
                let elements = if arguments.len() > 1 {
                    self.model_invoke(
                        site,
                        model,
                        arg(1),
                        arg(2),
                        vec![elements, FlowVar::Prim],
                        None,
                    )
                } else {
                    elements
                };
                self.store(result, "$elements".into(), elements);
            }
            Some(Operation::Concat | Operation::Collection) => {
                for &value in arguments {
                    self.store(result, "$elements".into(), value);
                    let elements = self.load(value, "$elements".into());
                    self.store(result, "$elements".into(), elements);
                    if matches!(method.operation, Some(Operation::Collection)) {
                        let key = self.load(elements, "exact:0".into());
                        let value = self.load(elements, "exact:1".into());
                        self.store(result, "$keys".into(), key);
                        self.store(result, "$elements".into(), value);
                    }
                }
            }
            Some(Operation::PromiseAll | Operation::PromiseAllSettled) => {
                let array = self.temp();
                self.flows.rows.push(FlowRow::Alloc {
                    dst: array,
                    object: FlowObject::Site((site.0, site.0)),
                });
                self.store(array, "$array".into(), FlowVar::Prim);
                self.store(array, "$spread".into(), FlowVar::Prim);
                let elements = self.load(arg(0), "$elements".into());
                let fulfilled = self.load(elements, "$fulfilled".into());
                let value = self.join(elements, fulfilled);
                if matches!(method.operation, Some(Operation::PromiseAllSettled)) {
                    let record = self.temp();
                    self.flows.rows.push(FlowRow::Alloc {
                        dst: record,
                        object: FlowObject::Site((site.1, site.1)),
                    });
                    self.store(record, "exact:value".into(), value);
                    let rejected = self.load(elements, "$rejected".into());
                    self.store(record, "exact:reason".into(), rejected);
                    self.store(record, "exact:status".into(), FlowVar::Prim);
                    self.store(array, "$elements".into(), record);
                } else {
                    self.store(array, "$elements".into(), value);
                }
                self.store(result, "$fulfilled".into(), array);
            }
            Some(Operation::Flatten) => {
                let elements = self.load(receiver, "$elements".into());
                let closure = self.temp();
                self.copy(closure, elements);
                let nested = self.load(closure, "$elements".into());
                self.copy(closure, nested);
                self.store(result, "$elements".into(), closure);
            }
            Some(Operation::EventRegister) => {
                if let Some(event) = event {
                    self.store(receiver, format!("$event:{event}").into(), arg(1));
                } else {
                    self.sink(arg(1));
                }
            }
            Some(Operation::EventEmit) => {
                if let Some(event) = event {
                    let callee = self.load(receiver, format!("$event:{event}").into());
                    self.model_invoke(
                        site,
                        model,
                        callee,
                        receiver,
                        arguments.iter().skip(1).copied().collect(),
                        None,
                    );
                }
            }
            Some(Operation::Executor) => {
                self.executor_values(result, arg(0));
            }
            None => {}
        }
        for store in &method.stores {
            let base = self.model_value(&store.base, receiver, arguments, result, &called);
            let value = self.model_value(&store.value, receiver, arguments, result, &called);
            self.store(
                base,
                if store.field.starts_with('$') {
                    store.field.as_str().into()
                } else {
                    format!("exact:{}", store.field).into()
                },
                value,
            );
        }
        let value = self.temp();
        for path in &method.returns {
            let returned = self.model_value(path, receiver, arguments, result, &called);
            self.copy(value, returned);
        }
        // Platform results remain open: name-based matching cannot prove that
        // the receiver has no user override, or that a callback is total.
        if !matches!(
            method.operation,
            Some(
                Operation::Create
                    | Operation::DefineProperty
                    | Operation::DefineProperties
                    | Operation::Assign
            )
        ) {
            self.copy(value, FlowVar::Open);
        }
        value
    }

    /// Follows only binding copies, not properties or calls, to inspect a
    /// syntactically available shape. Unknown shapes keep their escape guard.
    fn sources(&self, value: FlowVar) -> BTreeSet<FlowVar> {
        let mut seen = BTreeSet::from([value]);
        let mut queue = vec![value];
        while let Some(value) = queue.pop() {
            for &src in self.copy_sources.get(&value).into_iter().flatten() {
                if seen.insert(src) {
                    queue.push(src);
                }
            }
        }
        seen
    }

    pub(super) fn known_fields(&self, value: FlowVar) -> BTreeMap<Box<str>, Vec<FlowVar>> {
        let bases = self.sources(value);
        let mut fields = BTreeMap::<Box<str>, Vec<FlowVar>>::new();
        for base in bases {
            if let Some(stores) = self.field_sources.get(&base) {
                for (field, values) in stores {
                    fields.entry(field.clone()).or_default().extend(values);
                }
            }
        }
        fields
    }

    pub(super) fn array_arguments(&self, value: FlowVar) -> Option<Vec<FlowVar>> {
        let fields = self.known_fields(value);
        if !fields.contains_key("$array") || fields.contains_key("$spread") {
            return None;
        }
        let mut indexed = BTreeMap::new();
        for (field, values) in fields {
            if let Some(index) = field
                .strip_prefix("exact:")
                .and_then(|key| key.parse::<usize>().ok())
            {
                if values.len() != 1 {
                    return None;
                }
                indexed.insert(index, values[0]);
            }
        }
        if indexed.keys().copied().ne(0..indexed.len()) {
            return None;
        }
        Some(indexed.into_values().collect())
    }

    fn copy_fields(&mut self, target: FlowVar, source: FlowVar) {
        let fields = self.known_fields(source);
        if fields.is_empty() && source != FlowVar::Prim {
            self.sink(target);
            self.sink(source);
        }
        for (field, values) in fields {
            if field.starts_with("exact:") {
                for value in values {
                    self.store(target, field.clone(), value);
                }
            }
        }
    }

    fn define_descriptor(
        &mut self,
        site: FlowSpan,
        model: &str,
        target: FlowVar,
        field: &str,
        descriptor: FlowVar,
    ) {
        let fields = self.known_fields(descriptor);
        if let Some(values) = fields.get("exact:value") {
            for value in values {
                self.store(target, field.into(), *value);
            }
        }
        if let Some(getters) = fields.get("exact:get") {
            for &getter in getters {
                let returned = self.model_invoke(site, model, getter, target, Vec::new(), None);
                self.store(target, field.into(), returned);
            }
            self.sink(target);
        }
        if let Some(setters) = fields.get("exact:set") {
            for &setter in setters {
                self.sink(setter);
            }
            self.sink(target);
        }
        if fields.is_empty() {
            self.sink(target);
            self.sink(descriptor);
        }
    }

    pub(super) fn forward_bound_calls(
        &mut self,
        callee: FlowVar,
        site: FlowSpan,
        arguments: &[FlowVar],
        spread: Option<usize>,
    ) {
        let bound = self
            .sources(callee)
            .iter()
            .filter_map(|var| self.bound_values.get(var).cloned())
            .collect::<Vec<_>>();
        for binding in bound {
            let count = binding.prefix.len();
            let mut args = binding.prefix;
            args.extend_from_slice(arguments);
            self.model_invoke(
                site,
                "member:bind",
                binding.target,
                binding.receiver,
                args,
                spread.map(|position| position + count),
            );
        }
    }

    fn executor_values(&mut self, promise: FlowVar, executor: FlowVar) {
        let functions = self
            .sources(executor)
            .into_iter()
            .filter_map(|value| match value {
                FlowVar::Function(span) => Some(span),
                _ => None,
            })
            .collect::<Vec<_>>();
        for span in functions {
            for (index, field) in [(0, "$fulfilled"), (1, "$rejected")] {
                let parameter = FlowVar::Param(span, index);
                let calls = self
                    .flows
                    .calls
                    .iter()
                    .filter(|call| self.sources(call.callee).contains(&parameter))
                    .cloned()
                    .collect::<Vec<_>>();
                for call in calls {
                    if let Some(&value) = call.arguments.first() {
                        self.store(promise, field.into(), value);
                    }
                }
            }
        }
        self.store(promise, "$fulfilled".into(), FlowVar::Open);
        self.store(promise, "$rejected".into(), FlowVar::Open);
    }
}

fn static_length(expression: &Expression<'_>) -> Option<usize> {
    let Expression::ArrayExpression(array) = expression.get_inner_expression() else {
        return None;
    };
    array
        .elements
        .iter()
        .all(|element| {
            !matches!(
                element,
                ArrayExpressionElement::SpreadElement(_) | ArrayExpressionElement::Elision(_)
            )
        })
        .then_some(array.elements.len())
}

/// A direct literal cannot receive additional properties before the builtin
/// consumes it. Aliases and partial shapes must retain an escape guard.
fn literal_data_object(expression: &Expression<'_>) -> bool {
    let Expression::ObjectExpression(object) = expression.get_inner_expression() else {
        return false;
    };
    object.properties.iter().all(|property| matches!(property, ObjectPropertyKind::ObjectProperty(property) if property.kind == PropertyKind::Init && !property.computed))
}

fn literal_descriptor_map(expression: &Expression<'_>) -> bool {
    let Expression::ObjectExpression(object) = expression.get_inner_expression() else {
        return false;
    };
    literal_data_object(expression) && object.properties.iter().all(|property| matches!(property, ObjectPropertyKind::ObjectProperty(property) if literal_data_object(&property.value)))
}

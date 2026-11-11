// Exercises the bundled underscore UMD build across collections, arrays,
// objects, functions, chaining, templates and utilities.
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const _ = require("/bench/pkg/underscore-umd.js");

const step = (name, run) => {
  try {
    run();
  } catch (error) {
    console.error(`underscore driver step ${name}: ${error.message}`);
  }
};

const people = [{ name: "a", age: 3, team: "x" }, { name: "b", age: 1, team: "y" }, { name: "c", age: 2, team: "x" }];
step("collections", () => {
  _.each(people, (person) => person.name);
  _.each({ a: 1, b: 2 }, (value) => value);
  _.map(people, "name");
  _.reduce([1, 2, 3], (sum, value) => sum + value, 0);
  _.reduceRight([[1], [2]], (all, value) => all.concat(value), []);
  _.find(people, { team: "y" });
  _.filter(people, (person) => person.age > 1);
  _.where(people, { team: "x" });
  _.findWhere(people, { name: "c" });
  _.reject(people, { team: "x" });
  _.every([2, 4], (value) => value % 2 === 0);
  _.some([1, 3], (value) => value > 2);
  _.contains([1, 2], 2);
  _.invoke([[3, 1], [2, 1]], "sort");
  _.pluck(people, "age");
  _.max(people, "age");
  _.min([3, 1, 2]);
  _.sortBy(people, "age");
  _.groupBy(people, "team");
  _.indexBy(people, "name");
  _.countBy(people, (person) => person.team);
  _.shuffle([1, 2, 3]).length;
  _.sample([1, 2, 3], 2).length;
  _.toArray({ a: 1 });
  _.size({ a: 1, b: 2 });
  _.partition([1, 2, 3, 4], (value) => value % 2);
});
step("arrays", () => {
  _.first([1, 2, 3], 2);
  _.initial([1, 2, 3]);
  _.last([1, 2, 3]);
  _.rest([1, 2, 3]);
  _.compact([0, 1, false, 2]);
  _.flatten([1, [2, [3, [4]]]]);
  _.flatten([1, [2, [3]]], true);
  _.without([1, 2, 3], 2);
  _.union([1, 2], [2, 3]);
  _.intersection([1, 2, 3], [2, 3, 4]);
  _.difference([1, 2, 3], [2]);
  _.uniq([1, 2, 2, 3]);
  _.uniq(people, false, (person) => person.team);
  _.zip([1, 2], ["a", "b"]);
  _.unzip([[1, "a"], [2, "b"]]);
  _.object(["a", "b"], [1, 2]);
  _.chunk([1, 2, 3, 4, 5], 2);
  _.indexOf([1, 2, 3], 2);
  _.lastIndexOf([1, 2, 1], 1);
  _.sortedIndex([10, 20, 30], 25);
  _.findIndex(people, { name: "b" });
  _.findLastIndex(people, { team: "x" });
  _.range(0, 10, 3);
});
step("functions", () => {
  const bound = _.bind(function (greeting) { return `${greeting} ${this.name}`; }, { name: "x" }, "hi");
  bound();
  const object = { name: "o", greet() { return this.name; } };
  _.bindAll(object, "greet");
  object.greet();
  _.partial((a, b, c) => a + b + c, 1, _, 3)(2);
  const memo = _.memoize((n) => n * 2);
  memo(2);
  memo(2);
  _.throttle(() => 1, 0)();
  const debounced = _.debounce(() => 1, 0);
  debounced();
  debounced.cancel();
  const once = _.once(() => 1);
  once();
  once();
  _.after(2, () => 1)();
  _.before(2, () => 1)();
  _.wrap((value) => value, (inner, value) => inner(value) + 1)(1);
  _.negate((value) => value)(false);
  _.compose((value) => value + 1, (value) => value * 2)(3);
  _.restArguments((first, rest) => rest.length)(1, 2, 3);
});
step("objects", () => {
  const source = { a: 1, b: { c: 2 }, d: [1, 2] };
  _.keys(source);
  _.allKeys(source);
  _.values(source);
  _.mapObject(source, (value) => value);
  _.pairs(source);
  _.invert({ a: "x" });
  _.functions(_).length;
  _.findKey(source, (value) => value === 1);
  _.extend({}, source, { e: 3 });
  _.extendOwn({}, source);
  _.pick(source, "a", "b");
  _.pick(source, (value, key) => key === "a");
  _.omit(source, "a");
  _.defaults({ a: 9 }, source);
  _.create({ base: 1 }, { own: 2 });
  _.clone(source);
  _.tap(source, () => {});
  _.has(source, ["b", "c"]);
  _.get(source, ["b", "c"]);
  _.property("a")(source);
  _.propertyOf(source)("a");
  _.matcher({ a: 1 })(source);
  _.isEqual({ a: [1, { b: new Date(0) }] }, { a: [1, { b: new Date(0) }] });
  _.isEqual(new Map([[1, 2]]), new Map([[1, 2]]));
  _.isMatch(source, { a: 1 });
  [_.isEmpty, _.isElement, _.isArray, _.isObject, _.isArguments, _.isFunction, _.isString, _.isNumber, _.isFinite, _.isBoolean, _.isDate, _.isRegExp, _.isError, _.isSymbol, _.isMap, _.isWeakMap, _.isSet, _.isWeakSet, _.isArrayBuffer, _.isDataView, _.isTypedArray, _.isNaN, _.isNull, _.isUndefined].map((check) => check(source));
});
step("chaining and utilities", () => {
  _.chain(people).sortBy("age").map((person) => person.name).first(2).value();
  _(people).pluck("team");
  _([1, 2, 3]).chain().map((value) => value * 2).reverse().value();
  _.identity(1);
  _.constant(2)();
  _.noop();
  _.times(3, (n) => n);
  _.random(0, 0);
  _.uniqueId("bench");
  _.escape("<a href='x'>&</a>");
  _.unescape("&lt;b&gt;");
  _.result({ a: () => 1 }, "a");
  _.template("<% _.each(items, function (item) { %><b><%- item %></b><% }); %><%= title %>")({ items: ["x", "<y>"], title: "t" });
  _.template("{{ value }}", { interpolate: /\{\{(.+?)\}\}/g })({ value: 1 });
  _.mixin({ twice: (value) => value * 2 });
  _.twice(2);
  _.iteratee({ a: 1 })({ a: 1 });
  _.now() > 0;
});

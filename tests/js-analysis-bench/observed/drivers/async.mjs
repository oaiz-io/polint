// Exercises async's CommonJS collection, control-flow and utility functions
// with callbacks, promises and async functions.
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const async = require("/bench/pkg/index.js");

const square = (value, callback) => setImmediate(() => callback(null, value * value));
const isEven = (value, callback) => callback(null, value % 2 === 0);
const steps = [];
const run = (name, start) =>
  new Promise((resolve) => {
    try {
      start((error) => {
        if (error && error.message !== "bench") console.error(`async driver step ${name}: ${error.message}`);
        resolve();
      });
    } catch (error) {
      console.error(`async driver step ${name}: ${error.message}`);
      resolve();
    }
  });

steps.push(["map", (done) => async.map([1, 2, 3], square, done)]);
steps.push(["mapLimit", (done) => async.mapLimit([1, 2, 3, 4], 2, square, done)]);
steps.push(["mapSeries async fn", (done) => async.mapSeries([1, 2], async (value) => value + 1, done)]);
steps.push(["mapValues", (done) => async.mapValues({ a: 1, b: 2 }, (value, key, callback) => callback(null, value), done)]);
steps.push(["each", (done) => async.each([1, 2], (value, callback) => callback(), done)]);
steps.push(["eachOf", (done) => async.eachOf({ a: 1 }, (value, key, callback) => callback(), done)]);
steps.push(["eachSeries", (done) => async.eachSeries([1, 2], (value, callback) => callback(), done)]);
steps.push(["filter", (done) => async.filter([1, 2, 3, 4], isEven, done)]);
steps.push(["reject", (done) => async.reject([1, 2, 3], isEven, done)]);
steps.push(["detect", (done) => async.detect([1, 2, 3], isEven, done)]);
steps.push(["some", (done) => async.some([1, 3], isEven, done)]);
steps.push(["every", (done) => async.every([2, 4], isEven, done)]);
steps.push(["concat", (done) => async.concat([1, 2], (value, callback) => callback(null, [value, value]), done)]);
steps.push(["reduce", (done) => async.reduce([1, 2, 3], 0, (sum, value, callback) => callback(null, sum + value), done)]);
steps.push(["sortBy", (done) => async.sortBy([3, 1, 2], (value, callback) => callback(null, value), done)]);
steps.push(["groupBy", (done) => async.groupBy([1, 2, 3], (value, callback) => callback(null, value % 2), done)]);
steps.push(["times", (done) => async.times(3, (n, callback) => callback(null, n), done)]);
steps.push(["series", (done) => async.series([(callback) => callback(null, 1), (callback) => setImmediate(callback, null, 2)], done)]);
steps.push(["parallel", (done) => async.parallel({ a: (callback) => callback(null, 1), b: (callback) => callback(null, 2) }, done)]);
steps.push(["parallelLimit", (done) => async.parallelLimit([(callback) => callback(null, 1), (callback) => callback(null, 2)], 1, done)]);
steps.push(["waterfall", (done) => async.waterfall([(callback) => callback(null, 1), (value, callback) => callback(null, value + 1)], done)]);
steps.push(["auto", (done) => async.auto({ a: (callback) => callback(null, 1), b: ["a", (results, callback) => callback(null, results.a + 1)] }, done)]);
steps.push(["autoInject", (done) => async.autoInject({ a: (callback) => callback(null, 1), b: (a, callback) => callback(null, a + 1) }, done)]);
steps.push(["whilst", (done) => {
  let count = 0;
  async.whilst((callback) => callback(null, count < 3), (callback) => { count += 1; callback(); }, done);
}]);
steps.push(["until", (done) => {
  let count = 0;
  async.until((callback) => callback(null, count >= 2), (callback) => { count += 1; callback(); }, done);
}]);
steps.push(["retry", (done) => {
  let attempts = 0;
  async.retry({ times: 3, interval: 0 }, (callback) => { attempts += 1; callback(attempts < 2 ? new Error("again") : null, attempts); }, done);
}]);
steps.push(["queue", (done) => {
  const queue = async.queue((task, callback) => setImmediate(callback), 2);
  queue.push([1, 2, 3]);
  queue.drain(done);
}]);
steps.push(["priorityQueue", (done) => {
  const queue = async.priorityQueue((task, callback) => callback(), 1);
  queue.push(1, 2);
  queue.push(2, 1);
  queue.drain(done);
}]);
steps.push(["cargo", (done) => {
  const cargo = async.cargo((tasks, callback) => callback(), 2);
  cargo.push([1, 2, 3]);
  cargo.drain(done);
}]);
steps.push(["compose", (done) => async.compose((value, callback) => callback(null, value + 1), (value, callback) => callback(null, value * 2))(3, done)]);
steps.push(["seq", (done) => async.seq((value, callback) => callback(null, value + 1))(1, done)]);
steps.push(["applyEach", (done) => async.applyEach([(value, callback) => callback(null, value)], 1)(done)]);
steps.push(["memoize", (done) => {
  const slow = async.memoize((value, callback) => callback(null, value));
  slow(1, () => slow(1, done));
}]);
steps.push(["timeout", (done) => async.timeout((callback) => callback(null, 1), 1000)(done)]);
steps.push(["reflect", (done) => async.parallel([async.reflect((callback) => callback(new Error("bench")))], done)]);
steps.push(["tryEach", (done) => async.tryEach([(callback) => callback(new Error("bench")), (callback) => callback(null, 1)], done)]);
steps.push(["race", (done) => async.race([(callback) => callback(null, 1)], done)]);
steps.push(["asyncify", (done) => async.asyncify((value) => value + 1)(1, done)]);
steps.push(["constant", (done) => async.constant(1, 2)(done)]);
steps.push(["ensureAsync", (done) => async.ensureAsync((callback) => callback(null, 1))(done)]);
steps.push(["forever", (done) => {
  let count = 0;
  async.forever((next) => { count += 1; next(count > 2 ? new Error("bench") : null); }, (error) => done(error));
}]);
steps.push(["promise form", (done) => {
  async.map([1, 2], async (value) => value * 2).then(() => done(), done);
}]);
steps.push(["error path", (done) => async.map([1], (value, callback) => callback(new Error("bench")), done)]);

for (const [name, start] of steps) await run(name, start);

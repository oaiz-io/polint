// Exercises co as its readme documents it: co() over a generator function,
// co.wrap, every yieldable kind (promises, thunks, arrays, objects,
// generators and generator functions, nested), and errors caught inside the
// generator and through the returned promise.
import co from "/bench/pkg/index.js";

const step = async (name, run) => {
  try {
    await run();
  } catch (error) {
    console.error(`co driver step ${name}: ${error.message}`);
  }
};

const onerror = (error) => console.error(`co driver: ${error.message}`);

await step("promise", () =>
  co(function* () {
    return yield Promise.resolve(true);
  }).then((value) => value, onerror),
);
await step("wrap", () => {
  const fn = co.wrap(function* (value) {
    return yield Promise.resolve(value);
  });
  return fn(true);
});
await step("array", () =>
  co(function* () {
    return yield [Promise.resolve(1), Promise.resolve(2), Promise.resolve(3)];
  }).catch(onerror),
);
await step("object", () =>
  co(function* () {
    return yield { 1: Promise.resolve(1), 2: Promise.resolve(2), plain: 3 };
  }).catch(onerror),
);
await step("thunk", () =>
  co(function* () {
    const one = yield (done) => done(null, 1);
    const many = yield (done) => done(null, 2, 3);
    return [one, many];
  }).catch(onerror),
);
function* inner() {
  return yield Promise.resolve("inner");
}
await step("generator and generator function", () =>
  co(function* () {
    const fromGenerator = yield inner();
    const fromFunction = yield inner;
    return [fromGenerator, fromFunction];
  }).catch(onerror),
);
await step("nested", () =>
  co(function* () {
    return yield { list: [Promise.resolve(1), { deep: Promise.resolve(2) }], thunk: (done) => done(null, 3) };
  }).catch(onerror),
);
await step("caught inside", () =>
  co(function* () {
    try {
      yield Promise.reject(new Error("boom"));
    } catch (error) {
      return error.message;
    }
  }).catch(onerror),
);
await step("thunk error", () =>
  co(function* () {
    try {
      yield (done) => done(new Error("thunk boom"));
    } catch (error) {
      return error.message;
    }
  }).catch(onerror),
);
await step("rejected", () =>
  co(function* () {
    yield Promise.reject(new Error("uncaught"));
  }).catch((error) => error.message),
);
await step("thrown", () =>
  co(function* () {
    throw new Error("thrown");
  }).catch((error) => error.message),
);
await step("not yieldable", () =>
  co(function* () {
    yield 42;
  }).catch((error) => error.message),
);
await step("plain function", () => co(() => 7));
await step("generator object", () => co(inner()));
await step("arguments", () =>
  co(function* (a, b) {
    return a + b;
  }, 1, 2),
);

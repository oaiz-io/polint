// Runs node-forge's own node test suite from its release tag, as
// `npm run test-node` does: mocha over tests/unit/index.js, which loads the
// unit tests and tests/security. Every source of randomness the suite and
// forge read is replaced first by one seeded generator, so key generation,
// padding and nonces are the same in every run. The suite does not exercise
// forge's http, socket, xhr or worker modules over a network.
import crypto from "node:crypto";
import { createRequire } from "node:module";

// mulberry32: a small deterministic generator; not for cryptography.
let state = 0x5eed1234;
const next = () => {
  state = (state + 0x6d2b79f5) | 0;
  let t = Math.imul(state ^ (state >>> 15), 1 | state);
  t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
};
const fill = (view) => {
  const bytes = new Uint8Array(view.buffer, view.byteOffset, view.byteLength);
  for (let i = 0; i < bytes.length; i += 1) bytes[i] = Math.floor(next() * 256);
  return view;
};
Math.random = next;
crypto.randomBytes = (size, callback) => {
  const bytes = fill(Buffer.alloc(size));
  if (typeof callback !== "function") return bytes;
  process.nextTick(callback, null, bytes);
};
crypto.randomFillSync = (view) => fill(view);
// `crypto.getRandomValues` is a getter for the Web Crypto method, which is
// replaced on the shared instance.
Object.defineProperty(globalThis.crypto, "getRandomValues", { value: (view) => fill(view) });

const require = createRequire("/bench/pkg/package.json");
process.env.NODE_ENV = "test";
const Mocha = require("mocha");

function Quiet(runner) {
  runner.on("fail", (test, error) => {
    console.error(`node-forge suite: ${test.fullTitle()}: ${String(error.message).split("\n")[0]}`);
  });
}

const mocha = new Mocha({ reporter: Quiet, timeout: 600000 });
mocha.addFile("/bench/pkg/tests/unit/index.js");
const failures = await new Promise((resolve) => mocha.run(resolve));
console.error(`node-forge suite: ${failures} failing`);
process.exit(0);

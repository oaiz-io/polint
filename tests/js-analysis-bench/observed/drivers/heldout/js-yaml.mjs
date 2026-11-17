// Runs js-yaml's own test suite from its release tag, as `npm test` runs
// mocha: every test/*.js file. The property test draws its inputs from
// fast-check, which is given a fixed seed so two runs draw the same inputs.
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";

const require = createRequire("/bench/pkg/package.json");
const Mocha = require("mocha");
require("fast-check").configureGlobal({ seed: 20210114 });

function Quiet(runner) {
  runner.on("fail", (test, error) => {
    console.error(`js-yaml suite: ${test.fullTitle()}: ${String(error.message).split("\n")[0]}`);
  });
}

const mocha = new Mocha({ reporter: Quiet, timeout: 120000 });
const root = "/bench/pkg/test";
for (const name of fs.readdirSync(root).sort()) {
  if (name.endsWith(".js")) mocha.addFile(path.join(root, name));
}
const failures = await new Promise((resolve) => mocha.run(resolve));
console.error(`js-yaml suite: ${failures} failing`);
process.exit(0);

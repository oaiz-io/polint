// Runs express's own test suite from its release tag, as `npm test` does:
// mocha over test/ and test/acceptance/, with test/support/env required first
// and global leak checks on. The timeout is raised because every express
// function carries the recorder's probe.
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";

const require = createRequire("/bench/pkg/package.json");
require("/bench/pkg/test/support/env.js");
const Mocha = require("mocha");

// Reports failing tests only; mocha's reporters print timings.
function Quiet(runner) {
  runner.on("fail", (test, error) => {
    console.error(`express suite: ${test.fullTitle()}: ${String(error.message).split("\n")[0]}`);
  });
}

const mocha = new Mocha({ reporter: Quiet, checkLeaks: true, timeout: 120000 });
for (const dir of ["test", "test/acceptance"]) {
  const root = path.join("/bench/pkg", dir);
  for (const name of fs.readdirSync(root).sort()) {
    if (name.endsWith(".js")) mocha.addFile(path.join(root, name));
  }
}
const failures = await new Promise((resolve) => mocha.run(resolve));
console.error(`express suite: ${failures} failing`);
// As mocha's --exit: servers a test left open do not hold the run.
process.exit(0);

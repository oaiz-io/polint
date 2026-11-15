// Runtime call recorder for the JS/TS analysis benchmark.
//
// Loaded with `node --expose-internals --import <this file>` inside the
// benchmark sandbox (no network, no credentials, read-only package tree). It
// never runs as part of `cargo test`; `bench.py record-observed` drives it.
//
// Modes (BENCH_MODE):
//   calls    instrument every function in BENCH_FILES, run BENCH_DRIVER, and
//            record each observed call as (call site, callee) in UTF-8 byte
//            spans of the original source;
//   exports  load every file in BENCH_FILES without instrumentation and record
//            the names it exports.
//
// BENCH_FILES is a JSON list of paths relative to BENCH_ROOT.
//
// Output is one JSON document written to BENCH_OUT when the process exits.
//
// How a call is attributed. Every instrumented function starts with a probe.
// The probe captures structured stack frames: frame 0 is the callee. When
// frame 1 lies in an instrumented file, the frame position is mapped back to
// the original source and the smallest call, `new` or tagged-template
// expression whose callee region holds that position is the call site; a
// position outside any callee region is an implicit call (accessor, coercion,
// iterator protocol) and is recorded apart. When frame 1 is runtime code
// (a builtin such as Array.prototype.map, or node internals) and the next
// in-tree frame is a call site, the call is a callback invoked by the runtime
// on behalf of that site. Calls whose stack reaches no in-tree frame (driver
// calls, promise reactions, timers) are entries, not edges.

import { createRequire, registerHooks, stripTypeScriptTypes } from "node:module";
import { fileURLToPath, pathToFileURL } from "node:url";
import fs from "node:fs";
import path from "node:path";

const require = createRequire(import.meta.url);
const acorn = require("internal/deps/acorn/acorn/dist/acorn");

const ROOT = path.resolve(process.env.BENCH_ROOT);
const ROOT_URL = pathToFileURL(ROOT + path.sep).href;
const MODE = process.env.BENCH_MODE;
const OUT = process.env.BENCH_OUT;
const SOURCE_EXTENSIONS = new Set([".js", ".mjs", ".cjs", ".ts", ".mts", ".cts"]);
const PROBE = "__polintBenchEnter";
const SELECTED = new Set(JSON.parse(process.env.BENCH_FILES));

/** @type {Map<string, FileInfo>} instrumented file path -> mapping info */
const files = new Map();
/** @type {Array<{file: FileInfo, start: number, end: number, name: string}>} */
const functions = [];
const edges = new Map();
const failures = [];
const transformed = new Set();

function relative(file) {
  return path.relative(ROOT, file).split(path.sep).join("/");
}

function isTypeScript(file) {
  return /\.[cm]?ts$/.test(file);
}

// ---------------------------------------------------------------------------
// Resolution: TypeScript sources import siblings with a `.js` suffix.

registerHooks({
  resolve(specifier, context, nextResolve) {
    try {
      return nextResolve(specifier, context);
    } catch (error) {
      const parent = context.parentURL;
      if (
        parent &&
        parent.startsWith(ROOT_URL) &&
        specifier.startsWith(".") &&
        /\.[cm]?js$/.test(specifier)
      ) {
        const candidate = specifier.replace(/\.([cm]?)js$/, ".$1ts");
        return nextResolve(candidate, context);
      }
      throw error;
    }
  },
  load(url, context, nextLoad) {
    const loaded = nextLoad(url, context);
    if (!url.startsWith(ROOT_URL)) return loaded;
    const file = fileURLToPath(url);
    if (!SOURCE_EXTENSIONS.has(path.extname(file))) return loaded;
    const selected = SELECTED.has(relative(file));
    const original = fs.readFileSync(file, "utf8");
    let format = loaded.format;
    if (format === "module-typescript") format = "module";
    if (format === "commonjs-typescript") format = "commonjs";
    // A `require` of a `.js` file outside any `"type"` scope reports no format.
    if (format == null) format = file.endsWith(".mjs") ? "module" : "commonjs";
    let executable = original;
    let stripped = true;
    if (isTypeScript(file)) {
      try {
        executable = stripTypeScriptTypes(original, { mode: "strip" });
      } catch {
        // Enums, namespaces and parameter properties need code generation,
        // which moves every position; such a file runs but is not
        // instrumented, and its frames end attribution like driver frames.
        executable = stripTypeScriptTypes(original, { mode: "transform" });
        stripped = false;
        if (selected) transformed.add(relative(file));
      }
    }
    if (MODE === "calls" && selected && stripped && (format === "module" || format === "commonjs")) {
      try {
        executable = instrument(file, original, executable, format);
      } catch (error) {
        failures.push({ file: relative(file), stage: "instrument", error: String(error.message) });
      }
    }
    return { format, source: executable, shortCircuit: true };
  },
});

// ---------------------------------------------------------------------------
// Instrumentation.

class FileInfo {
  constructor(file, original, executable, ast) {
    this.rel = relative(file);
    this.original = original;
    this.ast = ast;
    // Sorted insertion points in original UTF-16 offsets, with lengths.
    this.inserts = [];
    this.instrumented = executable;
    this.lineStarts = null;
    this.byteCache = new Map();
  }

  // UTF-16 offset in the original text -> UTF-8 byte offset.
  byte(offset) {
    let cached = this.byteCache.get(offset);
    if (cached === undefined) {
      cached = Buffer.byteLength(this.original.slice(0, offset), "utf8");
      this.byteCache.set(offset, cached);
    }
    return cached;
  }

  // 1-based line and column in the instrumented text -> original UTF-16 offset.
  originalOffset(line, column) {
    if (this.lineStarts === null) {
      this.lineStarts = [0];
      const text = this.instrumented;
      for (let i = 0; i < text.length; i += 1) {
        if (text.charCodeAt(i) === 10) this.lineStarts.push(i + 1);
      }
    }
    const position = this.lineStarts[line - 1] + column - 1;
    // Undo every insertion that starts before this point; a point inside an
    // inserted probe maps to the insertion point.
    let removed = 0;
    for (const insert of this.inserts) {
      const startInOutput = insert.at + insert.shift;
      if (startInOutput >= position) break;
      removed += Math.min(insert.length, position - startInOutput);
    }
    return position - removed;
  }
}

function parse(source, format) {
  return acorn.parse(source, {
    ecmaVersion: "latest",
    sourceType: format === "module" ? "module" : "script",
    allowHashBang: true,
    allowReturnOutsideFunction: format !== "module",
    allowAwaitOutsideFunction: true,
  });
}

function walk(node, visit, parent = null) {
  if (!node || typeof node.type !== "string") return;
  visit(node, parent);
  for (const key of Object.keys(node)) {
    if (key === "type" || key === "start" || key === "end") continue;
    const child = node[key];
    if (Array.isArray(child)) {
      for (const item of child) if (item && typeof item.type === "string") walk(item, visit, node);
    } else if (child && typeof child.type === "string") {
      walk(child, visit, node);
    }
  }
}

function functionName(node, parent) {
  if (node.id && node.id.name) return node.id.name;
  if (!parent) return "";
  if ((parent.type === "MethodDefinition" || parent.type === "Property" || parent.type === "PropertyDefinition") && parent.key) {
    return parent.key.name ?? parent.key.value ?? "";
  }
  if (parent.type === "VariableDeclarator" && parent.id && parent.id.name) return parent.id.name;
  if (parent.type === "AssignmentExpression" && parent.left) {
    const left = parent.left;
    if (left.type === "Identifier") return left.name;
    if (left.type === "MemberExpression" && !left.computed) return left.property.name;
  }
  return "";
}

function instrument(file, original, executable, format) {
  const ast = parse(executable, format);
  const info = new FileInfo(file, original, executable, ast);
  const inserts = [];
  walk(ast, (node, parent) => {
    if (
      node.type !== "FunctionDeclaration" &&
      node.type !== "FunctionExpression" &&
      node.type !== "ArrowFunctionExpression"
    ) {
      return;
    }
    // A method's span starts at its key, which is where analysers put it.
    const owner =
      parent && (parent.type === "MethodDefinition" || (parent.type === "Property" && (parent.method || parent.kind !== "init")))
        ? parent
        : node;
    const id = functions.length;
    functions.push({ file: info, start: owner.start, end: node.end, name: functionName(node, parent) });
    const probe = `${PROBE}(${id});`;
    if (node.body.type === "BlockStatement") {
      let at = node.body.start + 1;
      for (const statement of node.body.body) {
        if (statement.type === "ExpressionStatement" && statement.directive !== undefined) {
          at = statement.end;
        } else {
          break;
        }
      }
      inserts.push({ at, text: probe });
    } else {
      inserts.push({ at: node.body.start, text: `(${PROBE}(${id}), ` });
      inserts.push({ at: node.body.end, text: ")" });
    }
  });
  inserts.sort((a, b) => a.at - b.at);
  let output = "";
  let cursor = 0;
  let shift = 0;
  for (const insert of inserts) {
    output += executable.slice(cursor, insert.at) + insert.text;
    info.inserts.push({ at: insert.at, shift, length: insert.text.length });
    shift += insert.text.length;
    cursor = insert.at;
  }
  output += executable.slice(cursor);
  info.instrumented = output;
  files.set(file, info);
  return output;
}

// ---------------------------------------------------------------------------
// Probe.

function syntaxOf(node) {
  if (node.type === "NewExpression") return "new";
  if (node.type === "TaggedTemplateExpression") return "tagged-template";
  const callee = node.callee;
  if (callee.type === "Identifier") return "identifier";
  if (callee.type === "Super") return "super";
  if (callee.type === "MemberExpression") {
    if (callee.object.type === "Super") return "super-member";
    return callee.computed ? "computed-member" : "member";
  }
  return "expression";
}

// The smallest call-like node whose callee region holds `offset`.
// V8 places a call whose callee is parenthesised, such as the `(0, fn)(x)`
// form that transpilers emit, at the argument list's opening parenthesis, so
// the callee region runs through closing parentheses and optional-chain
// punctuation up to and including that parenthesis (or a template's backtick).
function calleeRegionEnd(text, end) {
  let index = end;
  while (index < text.length && /[\s)?.]/.test(text[index])) index += 1;
  return text[index] === "(" || text[index] === "`" ? index : end;
}

function callSiteAt(info, offset) {
  let best = null;
  const visit = (node) => {
    if (!node || typeof node.type !== "string") return;
    if (offset < node.start || offset > node.end) return;
    const callee =
      node.type === "CallExpression" || node.type === "NewExpression"
        ? node.callee
        : node.type === "TaggedTemplateExpression"
          ? node.tag
          : null;
    if (callee && offset >= node.start && offset <= calleeRegionEnd(info.original, callee.end)) {
      if (!best || node.end - node.start <= best.end - best.start) best = node;
    }
    for (const key of Object.keys(node)) {
      if (key === "type" || key === "start" || key === "end") continue;
      const child = node[key];
      if (Array.isArray(child)) for (const item of child) visit(item);
      else if (child && typeof child.type === "string") visit(child);
    }
  };
  visit(info.ast);
  return best;
}

function spanKey(info, start, end) {
  return `${info.rel}@${info.byte(start)}..${info.byte(end)}`;
}

function inTreeFrame(frame) {
  const name = frame.getFileName();
  if (!name) return null;
  const file = name.startsWith("file:") ? fileURLToPath(name) : name;
  return files.get(file) ?? null;
}

function isDriverFrame(frame) {
  const name = frame.getFileName();
  return Boolean(name) && !name.startsWith("node:") && inTreeFrame(frame) === null;
}

function record(kind, site, syntax, fn) {
  const callee = `${fn.file.rel}@${fn.file.byte(fn.start)}..${fn.file.byte(fn.end)}`;
  const key = `${kind}\u0000${site}\u0000${callee}`;
  if (!edges.has(key)) edges.set(key, { kind, site, syntax, callee, callee_name: fn.name });
}

let busy = false;
globalThis[PROBE] = function enter(id) {
  if (busy) return;
  busy = true;
  try {
    const fn = functions[id];
    const holder = {};
    const limit = Error.stackTraceLimit;
    const prepare = Error.prepareStackTrace;
    Error.stackTraceLimit = 6;
    Error.prepareStackTrace = (_, frames) => frames;
    Error.captureStackTrace(holder, enter);
    const frames = holder.stack;
    Error.prepareStackTrace = prepare;
    Error.stackTraceLimit = limit;
    // frames[0] is the callee itself.
    let runtimeBetween = false;
    for (let i = 1; i < frames.length; i += 1) {
      const frame = frames[i];
      const info = inTreeFrame(frame);
      if (info === null) {
        if (isDriverFrame(frame)) break;
        runtimeBetween = true;
        continue;
      }
      const offset = info.originalOffset(frame.getLineNumber(), frame.getColumnNumber());
      const node = callSiteAt(info, offset);
      if (node === null) {
        if (!runtimeBetween) {
          record("implicit", `${info.rel}@${info.byte(offset)}`, "implicit", fn);
          return;
        }
        break;
      }
      record(runtimeBetween ? "callback" : "call", spanKey(info, node.start, node.end), syntaxOf(node), fn);
      return;
    }
  } finally {
    busy = false;
  }
};

// ---------------------------------------------------------------------------
// Export capture.

async function exportsOf(file) {
  try {
    const value = require(file);
    if (value && value[Symbol.toStringTag] === "Module") {
      return { system: "esm", names: Object.keys(value).sort() };
    }
    const names = value !== null && (typeof value === "object" || typeof value === "function") ? Object.keys(value) : [];
    const plain = value !== null && typeof value === "object" && Object.getPrototypeOf(value) === Object.prototype;
    return { system: "cjs", names: names.sort(), whole_value: plain ? "object" : typeof value };
  } catch (requireError) {
    try {
      const namespace = await import(pathToFileURL(file).href);
      return { system: "esm", names: Object.keys(namespace).sort() };
    } catch (importError) {
      return { error: String(importError.message ?? importError).split("\n")[0] };
    }
  }
}

// ---------------------------------------------------------------------------
// Output.

function writeCalls() {
  const order = (row) => `${row.kind}\u0000${row.site}\u0000${row.callee}`;
  // Code-unit order, independent of the host locale.
  const rows = [...edges.values()].sort((a, b) => (order(a) < order(b) ? -1 : order(a) > order(b) ? 1 : 0));
  const instrumented = [...files.values()].map((info) => info.rel).sort();
  const document = {
    schema: "polint-js-bench-observed-calls-1",
    node: process.version,
    instrumented_files: instrumented,
    instrumented_functions: functions.length,
    transformed_not_instrumented: [...transformed].sort(),
    failures,
    edges: rows,
  };
  fs.writeFileSync(OUT, JSON.stringify(document, null, 2) + "\n");
}

if (MODE === "calls") {
  process.on("exit", writeCalls);
  const driver = path.resolve(process.env.BENCH_DRIVER);
  process.on("uncaughtException", (error) => {
    failures.push({ file: relative(driver), stage: "driver", error: String(error.message) });
  });
  try {
    await import(pathToFileURL(driver).href);
  } catch (error) {
    failures.push({ file: relative(driver), stage: "driver", error: String(error.message) });
  }
} else if (MODE === "exports") {
  const result = {};
  for (const rel of [...SELECTED].sort()) result[rel] = await exportsOf(path.join(ROOT, rel));
  fs.writeFileSync(
    OUT,
    JSON.stringify(
      {
        schema: "polint-js-bench-observed-exports-1",
        node: process.version,
        transformed_not_instrumented: [...transformed].sort(),
        files: result,
      },
      null,
      2,
    ) + "\n",
  );
} else {
  throw new Error(`unknown BENCH_MODE ${MODE}`);
}

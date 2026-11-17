// Exercises acorn as its readme documents it: parse with the documented
// options (locations, ranges, onToken and onComment as callbacks and arrays,
// onInsertedSemicolon, onTrailingComma, program, sourceFile, preserveParens,
// the allow* options), parseExpressionAt, tokenizer with getToken and
// iteration, getLineInfo, the identifier predicates, Parser.extend, and
// syntax errors with their pos and loc. The input is a fixed corpus: the
// shipped JavaScript of other held-out packages, read in sorted order, each
// file parsed as a module and, when that fails, as a script.
import { readdirSync, readFileSync, statSync } from "node:fs";
import {
  Parser,
  getLineInfo,
  isIdentifierChar,
  isIdentifierStart,
  isNewLine,
  parse,
  parseExpressionAt,
  tokTypes,
  tokenizer,
  version,
} from "/bench/pkg/dist/acorn.mjs";

const CORPUS = "/bench/pkg/.polint-js-bench-corpus";

const step = (name, run) => {
  try {
    run();
  } catch (error) {
    console.error(`acorn driver step ${name}: ${error.message}`);
  }
};

const corpusFiles = (directory) =>
  readdirSync(directory)
    .sort()
    .flatMap((entry) => {
      const path = `${directory}/${entry}`;
      return statSync(path).isDirectory() ? corpusFiles(path) : [path];
    });

const parseFile = (source, path) => {
  const tokens = [];
  const comments = [];
  const options = {
    ecmaVersion: "latest",
    locations: true,
    ranges: true,
    sourceFile: path,
    onToken: tokens,
    onComment: comments,
  };
  try {
    return parse(source, { ...options, sourceType: "module" });
  } catch {
    tokens.length = 0;
    comments.length = 0;
    return parse(source, {
      ...options,
      sourceType: "script",
      allowReturnOutsideFunction: true,
      allowHashBang: true,
    });
  }
};

step("corpus", () => {
  for (const path of corpusFiles(CORPUS)) {
    const source = readFileSync(path, "utf8");
    try {
      const program = parseFile(source, path);
      if (program.type !== "Program") throw new Error(`${path}: not a program`);
    } catch (error) {
      console.error(`acorn driver corpus ${path}: ${error.message}`);
    }
  }
});

step("callbacks", () => {
  const source = "// line\nlet a = [1, 2,]\n/* block */ let b = { c: 1, }\na\n";
  const seen = [];
  parse(source, {
    ecmaVersion: 2020,
    locations: true,
    onToken: (token) => seen.push(token.type.label),
    onComment: (block, text, start, end, startLoc, endLoc) => seen.push([block, text, start, end, startLoc.line, endLoc.line]),
    onInsertedSemicolon: (position, loc) => seen.push(["semicolon", position, loc.line]),
    onTrailingComma: (position, loc) => seen.push(["comma", position, loc.line]),
  });
});

step("program", () => {
  const first = parse("var a = 1;", { ecmaVersion: 2020, sourceType: "script", directSourceFile: "a.js" });
  parse("var b = a + 1;", { ecmaVersion: 2020, sourceType: "script", program: first, directSourceFile: "b.js" });
  if (first.body.length !== 2) throw new Error("program did not collect both files");
});

step("versions", () => {
  parse("var let = 1; with (a) {}", { ecmaVersion: 5, sourceType: "script" });
  parse("var x = { class: 1, if: 2 }", { ecmaVersion: 3, allowReserved: true });
  parse("return 1", { ecmaVersion: 2015, allowReturnOutsideFunction: true });
  parse("function f() { import('a'); export default 1 }", {
    ecmaVersion: 2020,
    sourceType: "module",
    allowImportExportEverywhere: true,
  });
  parse("await Promise.resolve(1)", { ecmaVersion: 2017, allowAwaitOutsideFunction: true });
  parse("super.method()", { ecmaVersion: 2015, allowSuperOutsideMethod: true });
  parse("#!/usr/bin/env node\nvar a = 1", { ecmaVersion: 2020, allowHashBang: true });
  parse("exports.a = 1; return", { ecmaVersion: 2020, sourceType: "commonjs" });
  parse("'use strict'; var a = (1 + 2) * (3)", { ecmaVersion: 2020, strict: true, preserveParens: true });
  parse("class A { m() { this.#x } }", { ecmaVersion: 2022, checkPrivateFields: false });
});

step("modern syntax", () => {
  const snippets = [
    "class A { #x = 1; static #y; static { this.z = A.#y } get #g() { return this.#x } has(o) { return #x in o } }",
    "a?.b?.[c]?.(d) ?? e",
    "a ||= b; c &&= d; e ??= f",
    "const n = 1_000_000n + 0xff_ffn",
    "/[\\p{L}--\\p{Ll}]/v.test(s)",
    "import json from './a.json' with { type: 'json' }",
    "export * as ns from 'mod'; export { default as d } from 'mod'",
    "await import('a'); const { a, ...rest } = await b",
    "label: for await (const x of y) { continue label }",
    "async function* g() { yield* h(); for (const [k, v] of Object.entries(o)) yield `${k}=${v}` }",
    "using res = getResource()",
    "try { f() } catch { g() } finally { h() }",
    "const re = /(?<year>\\d{4})-(?<month>\\d{2})/dgsu",
    "function F() { return new.target } import.meta.url",
  ];
  for (const source of snippets) {
    try {
      parse(source, { ecmaVersion: "latest", sourceType: "module", locations: true });
    } catch (error) {
      console.error(`acorn driver snippet ${JSON.stringify(source)}: ${error.message}`);
    }
  }
});

step("syntax errors", () => {
  const broken = [
    "var = 1",
    "function (",
    "let let = 1",
    "a => { yield 1 }",
    "class A { constructor() {} constructor() {} }",
    "'use strict'; with (a) {}",
    "for (let x of y, z) {}",
    "`unterminated",
    "/unterminated",
    "0b12",
    "({a:1} = 1)",
    "import a from 'b'",
    "return 1",
    "new.target",
    "class A { m() { this.#missing } }",
    "\\u{110000}",
  ];
  for (const source of broken) {
    try {
      parse(source, { ecmaVersion: "latest", sourceType: "script", locations: true });
      console.error(`acorn driver: ${JSON.stringify(source)} parsed`);
    } catch (error) {
      if (!(error instanceof SyntaxError) || typeof error.pos !== "number" || !error.loc) throw error;
    }
  }
});

step("expressions", () => {
  const source = "let x = a + b * c; f(g, h)";
  const expression = parseExpressionAt(source, 8, { ecmaVersion: 2020, locations: true });
  if (expression.end !== 17) throw new Error("parseExpressionAt stopped elsewhere");
  parseExpressionAt(source, 19, { ecmaVersion: 2020, startLocation: { line: 1, column: 19 }, locations: true });
  Parser.parseExpressionAt("(x) => x * 2", 0, { ecmaVersion: 2020 });
});

step("tokenizer", () => {
  const source = "const a = /re/g.test(b) ? `t${c}` : d / 2; // tail";
  const stream = tokenizer(source, { ecmaVersion: 2020, locations: true, ranges: true });
  const labels = [];
  for (let token = stream.getToken(); token.type !== tokTypes.eof; token = stream.getToken()) {
    labels.push(token.type.label);
  }
  const iterated = [...tokenizer(source, { ecmaVersion: 2020 })];
  if (iterated.length !== labels.length) throw new Error("iteration and getToken differ");
  for (const token of Parser.tokenizer("a + 1", { ecmaVersion: 2020 })) token.value;
});

step("line info", () => {
  const source = "a\nb\r\nc\u2028d";
  for (let offset = 0; offset < source.length; offset++) getLineInfo(source, offset);
  for (let index = 0; index < source.length; index++) isNewLine(source.charCodeAt(index));
});

step("identifiers", () => {
  for (const text of ["a", "$", "_", "é", "ℵ", "1", "-", "\u{1d400}"]) {
    const code = text.codePointAt(0);
    isIdentifierStart(code, true);
    isIdentifierStart(code, false);
    isIdentifierChar(code, true);
  }
});

step("extend", () => {
  // A plugin is a function from a Parser class to a subclass of it.
  const countNumbers = (Base) =>
    class extends Base {
      readNumber(startsWithDot) {
        this.numbers = (this.numbers || 0) + 1;
        return super.readNumber(startsWithDot);
      }
    };
  const traceIdentifiers = (Base) =>
    class extends Base {
      parseIdent(liberal) {
        const node = super.parseIdent(liberal);
        node.traced = true;
        return node;
      }
    };
  const Extended = Parser.extend(countNumbers, traceIdentifiers);
  Extended.parse("var a = 1 + .5 + b", { ecmaVersion: 2020 });
  const parser = new Extended({ ecmaVersion: 2020 }, "c * 2");
  parser.parse();
  if (typeof version !== "string") throw new Error("no version");
});

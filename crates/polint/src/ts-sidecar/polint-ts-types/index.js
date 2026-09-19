#!/usr/bin/env node
// @ts-check
'use strict';

// polint TypeScript type sidecar.
//
// Emits `polint-ts-types-1` NDJSON describing, per tsconfig project, the
// callables, call sites, resolved callees and receiver types the TypeScript
// type checker sees. The Rust side turns those rows into a type-directed
// call-graph tier; this process answers type questions and nothing else.
//
// Two invariants shape the whole file:
//
//   1. The program is built over the full import closure, but rows are emitted
//      only for files the caller listed in --scope-files. A checker that
//      cannot see an import cannot type a call, so narrowing the program would
//      change answers; narrowing emission only removes rows the caller would
//      drop anyway.
//   2. Positions cross the wire as UTF-8 byte offsets. TypeScript counts
//      UTF-16 code units, and the Rust facts this joins against count bytes,
//      so every position is converted.

const fs = require('fs');
const path = require('path');

const SCHEMA = 'polint-ts-types-1';
const PRINTED_TYPE_LIMIT = 512;
const FLUSH_ROWS = 256;

/** @typedef {Record<string, unknown>} Row */

/**
 * What one invocation has already put on the wire.
 *
 * Projects overlap: a file listed by both `tsconfig.json` and
 * `tsconfig.build.json` is compiled by both, and a solution config reaches the
 * same project through two references. Row identities are keyed on file and
 * offset, not on the project that reported them, so a session-wide record of
 * what has been emitted is what keeps one identity from crossing the wire
 * twice.
 *
 * @typedef {{visitedProjects: Set<string>, emittedFiles: Set<string>, emittedCallables: Set<string>}} Session
 */

function usage() {
  process.stderr.write(
    'usage: node index.js --root <path> --projects <comma-list> ' +
      '--typescript <module-dir> [--scope-files <path>] --ndjson\n'
  );
}

/**
 * @param {string[]} argv
 * @returns {{root: string, projects: string[], typescript: string, scopeFiles: string, ndjson: boolean}}
 */
function parseArgs(argv) {
  const parsed = {
    root: '.',
    projects: /** @type {string[]} */ ([]),
    typescript: '',
    scopeFiles: '',
    ndjson: false,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index];
    const value = () => {
      const next = argv[index + 1];
      if (next === undefined) {
        throw new Error(`${flag} requires a value`);
      }
      index += 1;
      return next;
    };
    switch (flag) {
      case '--root':
        parsed.root = value();
        break;
      case '--projects':
        parsed.projects = splitList(value());
        break;
      case '--typescript':
        parsed.typescript = value();
        break;
      case '--scope-files':
        parsed.scopeFiles = value();
        break;
      case '--ndjson':
        parsed.ndjson = true;
        break;
      default:
        throw new Error(`unknown flag ${flag}`);
    }
  }
  return parsed;
}

/**
 * @param {string} value
 * @returns {string[]}
 */
function splitList(value) {
  return value
    .split(',')
    .map((part) => part.trim())
    .filter((part) => part.length > 0);
}

/**
 * Loads the TypeScript module the caller resolved.
 *
 * The caller owns resolution because the compiler version is part of the
 * cache key and has to be known before this process starts.
 *
 * @param {string} location
 * @returns {any}
 */
function loadTypeScript(location) {
  // `require` resolves a relative specifier against this file, not against the
  // working directory, so a relative location has to be made absolute first.
  const resolved = path.resolve(process.cwd(), location);
  const candidates = [
    path.join(resolved, 'lib', 'typescript.js'),
    path.join(resolved, 'typescript.js'),
    resolved,
  ];
  let lastError = null;
  for (const candidate of candidates) {
    try {
      // eslint-disable-next-line global-require
      return require(candidate);
    } catch (error) {
      lastError = error;
    }
  }
  throw new Error(
    `could not load the TypeScript compiler from ${location}: ${
      lastError instanceof Error ? lastError.message : String(lastError)
    }`
  );
}

/**
 * Maps UTF-16 code-unit offsets to UTF-8 byte offsets for one file's text.
 *
 * ASCII-only text, which is the common case, takes the identity fast path. For
 * anything else the extra bytes contributed by each wide unit are accumulated
 * once and answered by binary search, so a file with astral-plane characters
 * still joins against byte-offset facts after the first such character.
 *
 * @param {string} text
 * @returns {(position: number) => number}
 */
function byteOffsetMapper(text) {
  let wide = false;
  for (let index = 0; index < text.length; index += 1) {
    if (text.charCodeAt(index) > 0x7f) {
      wide = true;
      break;
    }
  }
  if (!wide) {
    return (position) => position;
  }

  /** @type {number[]} */
  const breakpoints = [];
  /** @type {number[]} */
  const extras = [];
  let extra = 0;
  for (let index = 0; index < text.length; index += 1) {
    const bytes = unitByteLength(text, index);
    if (bytes > 1) {
      extra += bytes - 1;
      breakpoints.push(index);
      extras.push(extra);
    }
  }

  return (position) => {
    if (position <= 0) {
      return 0;
    }
    // The last wide unit that ends at or before `position` decides how many
    // extra bytes precede it.
    let low = 0;
    let high = breakpoints.length - 1;
    let found = -1;
    while (low <= high) {
      const middle = (low + high) >> 1;
      if (breakpoints[middle] < position) {
        found = middle;
        low = middle + 1;
      } else {
        high = middle - 1;
      }
    }
    return position + (found >= 0 ? extras[found] : 0);
  };
}

/**
 * UTF-8 byte length contributed by the code unit at `index`.
 *
 * A surrogate pair costs four bytes and spans two units, so each unit of the
 * pair is charged two. A lone surrogate is charged the three bytes the
 * replacement character costs, which is what Node writes for it.
 *
 * @param {string} text
 * @param {number} index
 * @returns {number}
 */
function unitByteLength(text, index) {
  const code = text.charCodeAt(index);
  if (code <= 0x7f) {
    return 1;
  }
  if (code <= 0x7ff) {
    return 2;
  }
  if (code >= 0xd800 && code <= 0xdbff) {
    const next = index + 1 < text.length ? text.charCodeAt(index + 1) : 0;
    return next >= 0xdc00 && next <= 0xdfff ? 2 : 3;
  }
  if (code >= 0xdc00 && code <= 0xdfff) {
    const previous = index > 0 ? text.charCodeAt(index - 1) : 0;
    return previous >= 0xd800 && previous <= 0xdbff ? 2 : 3;
  }
  return 3;
}

/**
 * Length-prefixed key, matching the Go sidecar's recipe so no separator can be
 * forged out of content.
 *
 * @param {string[]} parts
 * @returns {string}
 */
function stableKey(parts) {
  let key = '';
  for (const part of parts) {
    key += `${part.length}:${part};`;
  }
  return key;
}

class Emitter {
  /** @param {NodeJS.WriteStream} stream */
  constructor(stream) {
    this.stream = stream;
    /** @type {string[]} */
    this.pending = [];
    this.rowsEmitted = 0;
  }

  /** @param {Row} row */
  write(row) {
    row.schema = SCHEMA;
    this.pending.push(`${JSON.stringify(row)}\n`);
    if (this.pending.length >= FLUSH_ROWS) {
      this.flush();
    }
  }

  /** @param {Row} row */
  writeCounted(row) {
    this.rowsEmitted += 1;
    this.write(row);
  }

  flush() {
    if (this.pending.length === 0) {
      return;
    }
    this.stream.write(this.pending.join(''));
    this.pending = [];
  }
}

class PhaseTimer {
  constructor() {
    this.started = Date.now();
    this.stage = Date.now();
  }

  /** @returns {number} */
  closeStage() {
    const now = Date.now();
    const elapsed = now - this.stage;
    this.stage = now;
    return elapsed;
  }

  /** @returns {number} */
  total() {
    return Date.now() - this.started;
  }
}

function heapBytes() {
  try {
    return process.memoryUsage().heapUsed;
  } catch (error) {
    return 0;
  }
}

/**
 * @param {string} root
 * @param {string} absolute
 * @returns {string}
 */
function relativePath(root, absolute) {
  const relative = path.relative(root, absolute);
  return relative.split(path.sep).join('/');
}

/**
 * @param {string} scopeFilesPath
 * @returns {Set<string> | null}
 */
function readScopeFiles(scopeFilesPath) {
  // An empty path means the caller did not narrow the scan, which is not the
  // same as narrowing it to nothing: a null scope disables the filter.
  if (scopeFilesPath === '') {
    return null;
  }
  const raw = fs.readFileSync(scopeFilesPath, 'utf8');
  const scope = new Set();
  for (const line of raw.split('\n')) {
    const trimmed = line.trim();
    if (trimmed.length > 0) {
      scope.add(trimmed);
    }
  }
  return scope;
}

/**
 * Package-relative moniker for a declaration the scan does not own.
 *
 * Declarations under `node_modules` and the TypeScript lib files are named
 * rather than emitted as callables, so nothing outside the scan crosses the
 * wire as a full row.
 *
 * @param {string} fileName
 * @param {string} name
 * @returns {string}
 */
function externalMoniker(fileName, name) {
  const normalized = fileName.split(path.sep).join('/');
  // The standard library is named as the library it is, before the package it
  // happens to be installed under: the compiler is normally the repository's
  // own `node_modules/typescript`, so checking the package first would report
  // every `Array.prototype.map` call as a call into that package.
  const base = path.basename(normalized);
  if (/^lib\..*\.d\.ts$/.test(base) || base === 'lib.d.ts') {
    return `lib:${base}#${name}`;
  }
  const marker = '/node_modules/';
  const lastIndex = normalized.lastIndexOf(marker);
  if (lastIndex >= 0) {
    const tail = normalized.slice(lastIndex + marker.length);
    const segments = tail.split('/');
    const packageName = segments[0].startsWith('@')
      ? segments.slice(0, 2).join('/')
      : segments[0];
    const subpath = tail.slice(packageName.length).replace(/^\//, '');
    return `node_modules:${packageName}${subpath === '' ? '' : `/${subpath}`}#${name}`;
  }
  return `external:${base}#${name}`;
}

/**
 * @param {any} ts
 * @param {any} node
 * @returns {string}
 */
function callableKind(ts, node) {
  if (ts.isConstructorDeclaration(node)) {
    return 'constructor';
  }
  if (ts.isMethodDeclaration(node) || ts.isMethodSignature(node)) {
    return 'method';
  }
  if (ts.isGetAccessor(node)) {
    return 'getter';
  }
  if (ts.isSetAccessor(node)) {
    return 'setter';
  }
  if (ts.isArrowFunction(node)) {
    return 'arrow';
  }
  if (ts.isClassDeclaration(node) || ts.isClassExpression(node)) {
    return 'class';
  }
  return 'function';
}

/**
 * @param {any} ts
 * @param {any} node
 * @returns {string}
 */
function callableName(ts, node) {
  if (node.name !== undefined && node.name !== null) {
    if (ts.isIdentifier(node.name) || ts.isPrivateIdentifier(node.name)) {
      return String(node.name.escapedText);
    }
    if (ts.isStringLiteral(node.name) || ts.isNumericLiteral(node.name)) {
      return String(node.name.text);
    }
  }
  if (ts.isConstructorDeclaration(node)) {
    return 'constructor';
  }
  // An anonymous callable assigned to a name takes that name, which is what a
  // reader of the call graph expects to see.
  const parent = node.parent;
  if (parent !== undefined && parent !== null) {
    if (ts.isVariableDeclaration(parent) && ts.isIdentifier(parent.name)) {
      return String(parent.name.escapedText);
    }
    if (ts.isPropertyAssignment(parent) && ts.isIdentifier(parent.name)) {
      return String(parent.name.escapedText);
    }
    if (ts.isPropertyDeclaration(parent) && ts.isIdentifier(parent.name)) {
      return String(parent.name.escapedText);
    }
  }
  return '';
}

/**
 * @param {any} ts
 * @param {any} node
 * @returns {string}
 */
function callKind(ts, node) {
  if (ts.isNewExpression(node)) {
    return 'new';
  }
  if (ts.isTaggedTemplateExpression(node)) {
    return 'tagged_template';
  }
  if (ts.isDecorator(node)) {
    return 'decorator';
  }
  const callee = node.expression;
  if (
    callee !== undefined &&
    (ts.isPropertyAccessExpression(callee) || ts.isElementAccessExpression(callee))
  ) {
    return 'method';
  }
  return 'call';
}

/**
 * The expression whose type decides dispatch: the receiver for a member call,
 * the callee itself for a value call.
 *
 * @param {any} ts
 * @param {any} node
 * @returns {any}
 */
function dispatchExpression(ts, node) {
  if (ts.isTaggedTemplateExpression(node)) {
    return node.tag;
  }
  if (ts.isDecorator(node)) {
    return node.expression;
  }
  const callee = node.expression;
  if (callee === undefined || callee === null) {
    return undefined;
  }
  if (ts.isPropertyAccessExpression(callee) || ts.isElementAccessExpression(callee)) {
    return callee.expression;
  }
  return callee;
}

class ProjectEmitter {
  /**
   * @param {{ts: any, emitter: Emitter, root: string, scope: Set<string> | null, project: string, emittedCallables: Set<string>}} options
   */
  constructor(options) {
    this.ts = options.ts;
    this.emitter = options.emitter;
    this.root = options.root;
    this.scope = options.scope;
    this.project = options.project;
    /**
     * Callable identities already on the wire, shared across every project in
     * this session. Projects overlap — a file listed by both `tsconfig.json`
     * and `tsconfig.build.json` is compiled by both — and identities are keyed
     * on file and offset, so emitting per project would put the same key on
     * the wire twice and the reader would count a healthy repository's rows as
     * dropped duplicates.
     * @type {Set<string>}
     */
    this.emittedCallables = options.emittedCallables;
    /** @type {Map<string, Row>} */
    this.callables = new Map();
    /** @type {Map<string, (position: number) => number>} */
    this.offsetMappers = new Map();
    /**
     * In-scope class declarations by identity, with their instance methods by
     * name. The dispatch expansion needs both.
     * @type {Map<string, {node: any, sourceFile: any, methods: Map<string, any>}>}
     */
    this.classes = new Map();
    /**
     * Classes this project instantiates somewhere in scope. A declared type
     * with no instantiated implementation cannot be the target of a call at
     * run time, so the expansion is filtered by this set — the discriminant
     * that separates rapid-type from coarse class-hierarchy candidates.
     * @type {Set<string>}
     */
    this.instantiated = new Set();
  }

  /**
   * @param {any} sourceFile
   * @returns {(position: number) => number}
   */
  mapperFor(sourceFile) {
    const existing = this.offsetMappers.get(sourceFile.fileName);
    if (existing !== undefined) {
      return existing;
    }
    const mapper = byteOffsetMapper(sourceFile.text);
    this.offsetMappers.set(sourceFile.fileName, mapper);
    return mapper;
  }

  /**
   * @param {any} sourceFile
   * @param {number} start
   * @param {number} end
   * @returns {Row}
   */
  span(sourceFile, start, end) {
    const mapper = this.mapperFor(sourceFile);
    const startPosition = sourceFile.getLineAndCharacterOfPosition(start);
    const endPosition = sourceFile.getLineAndCharacterOfPosition(end);
    return {
      start_byte: mapper(start),
      end_byte: mapper(end),
      start_line: startPosition.line + 1,
      start_column: startPosition.character + 1,
      end_line: endPosition.line + 1,
      end_column: endPosition.character + 1,
    };
  }

  /**
   * @param {any} node
   * @param {any} sourceFile
   * @returns {string}
   */
  callableIdentity(node, sourceFile) {
    const relative = relativePath(this.root, sourceFile.fileName);
    const start = this.mapperFor(sourceFile)(node.getStart(sourceFile));
    return `${relative}:${start}:${callableName(this.ts, node)}`;
  }

  /**
   * Registers a callable row, deduplicated by identity.
   *
   * Call sites resolve to declarations that the walk may not have reached yet,
   * so registration is idempotent and the rows are emitted once at the end of
   * the project.
   *
   * @param {any} node
   * @param {any} sourceFile
   * @returns {string}
   */
  registerCallable(node, sourceFile) {
    const identity = this.callableIdentity(node, sourceFile);
    if (this.callables.has(identity) || this.emittedCallables.has(identity)) {
      return identity;
    }
    const ts = this.ts;
    const start = node.getStart(sourceFile);
    const nameNode = node.name !== undefined && node.name !== null ? node.name : undefined;
    const row = {
      kind: 'callable',
      project: this.project,
      callable: identity,
      name: callableName(ts, node),
      file: relativePath(this.root, sourceFile.fileName),
      span: this.span(sourceFile, start, node.end),
      name_span:
        nameNode === undefined
          ? null
          : this.span(sourceFile, nameNode.getStart(sourceFile), nameNode.end),
      callable_kind: callableKind(ts, node),
      stable_key: stableKey(['callable', identity]),
    };
    this.callables.set(identity, row);
    return identity;
  }

  flushCallables() {
    for (const [identity, row] of this.callables) {
      this.emitter.writeCounted(row);
      this.emittedCallables.add(identity);
    }
    this.callables.clear();
  }
}

/**
 * @param {any} ts
 * @param {any} node
 * @returns {boolean}
 */
function isCallLike(ts, node) {
  return (
    ts.isCallExpression(node) ||
    ts.isNewExpression(node) ||
    ts.isTaggedTemplateExpression(node) ||
    ts.isDecorator(node)
  );
}

/**
 * @param {any} ts
 * @param {any} node
 * @returns {boolean}
 */
function isCallableDeclaration(ts, node) {
  return (
    ts.isFunctionDeclaration(node) ||
    ts.isFunctionExpression(node) ||
    ts.isArrowFunction(node) ||
    ts.isMethodDeclaration(node) ||
    ts.isConstructorDeclaration(node) ||
    ts.isGetAccessor(node) ||
    ts.isSetAccessor(node)
  );
}

/**
 * A declaration that describes a call but never runs: an interface or type
 * member, an overload signature with no body, or an abstract method.
 *
 * `getResolvedSignature` answers with these whenever the receiver is typed by
 * an interface, so a typed tier that stopped there would name a declaration
 * that has no body to enter.
 *
 * @param {any} ts
 * @param {any} node
 * @returns {boolean}
 */
function isTypeLevelDeclaration(ts, node) {
  if (ts.isMethodSignature(node) || ts.isCallSignatureDeclaration(node)) {
    return true;
  }
  if (ts.isConstructSignatureDeclaration(node) || ts.isFunctionTypeNode(node)) {
    return true;
  }
  if (ts.isIndexSignatureDeclaration(node)) {
    return true;
  }
  const abstract =
    node.modifiers !== undefined &&
    node.modifiers !== null &&
    node.modifiers.some(
      /** @param {any} modifier */ (modifier) => modifier.kind === ts.SyntaxKind.AbstractKeyword
    );
  if (abstract) {
    return true;
  }
  // An overload signature (`function f(a: string): void;`) has no body, so it
  // declares the call without implementing it.
  return (
    (ts.isFunctionDeclaration(node) || ts.isMethodDeclaration(node)) &&
    (node.body === undefined || node.body === null)
  );
}

/**
 * @param {any} ts
 * @param {any} node
 * @param {any} sourceFile
 * @returns {any}
 */
function enclosingCallable(ts, node, sourceFile) {
  let current = node.parent;
  while (current !== undefined && current !== null && current !== sourceFile) {
    if (isCallableDeclaration(ts, current)) {
      return current;
    }
    current = current.parent;
  }
  return undefined;
}

/**
 * Declarations a resolved signature points at, or the class the `new` targets
 * when the class declares no constructor.
 *
 * @param {any} ts
 * @param {any} checker
 * @param {any} node
 * @returns {any[]}
 */
function calleeDeclarations(ts, checker, node) {
  /** @type {any[]} */
  const declarations = [];
  let signature;
  try {
    signature = checker.getResolvedSignature(node);
  } catch (error) {
    signature = undefined;
  }
  if (signature !== undefined && signature.declaration !== undefined) {
    declarations.push(signature.declaration);
  }
  if (declarations.length === 0) {
    const dispatch = dispatchExpression(ts, node);
    if (dispatch !== undefined) {
      let symbol;
      try {
        symbol = checker.getSymbolAtLocation(dispatch);
      } catch (error) {
        symbol = undefined;
      }
      if (symbol !== undefined && symbol !== null) {
        const aliased =
          (symbol.flags & ts.SymbolFlags.Alias) !== 0 ? tryAliasedSymbol(checker, symbol) : symbol;
        for (const declaration of (aliased && aliased.declarations) || []) {
          if (isCallableDeclaration(ts, declaration) || ts.isClassDeclaration(declaration)) {
            declarations.push(declaration);
          } else if (
            ts.isVariableDeclaration(declaration) &&
            declaration.initializer !== undefined &&
            isCallableDeclaration(ts, declaration.initializer)
          ) {
            declarations.push(declaration.initializer);
          }
        }
      }
    }
  }
  return declarations;
}

/**
 * @param {any} checker
 * @param {any} symbol
 * @returns {any}
 */
function tryAliasedSymbol(checker, symbol) {
  try {
    return checker.getAliasedSymbol(symbol);
  } catch (error) {
    return symbol;
  }
}

/**
 * @param {string} root
 * @param {string} fileName
 * @returns {boolean}
 */
function isOwnedByScan(root, fileName) {
  const normalized = fileName.split(path.sep).join('/');
  if (normalized.includes('/node_modules/')) {
    return false;
  }
  const relative = relativePath(root, fileName);
  return !relative.startsWith('../') && relative !== '';
}

/**
 * @param {{ts: any, emitter: Emitter, root: string, scope: Set<string> | null, projectPath: string, timer: PhaseTimer, totals: {projects: number, files: number}, session: Session}} options
 */
function emitProject(options) {
  const {ts, emitter, root, scope, projectPath, timer, totals, session} = options;
  const absoluteConfig = path.resolve(root, projectPath);
  if (session.visitedProjects.has(absoluteConfig)) {
    return;
  }
  session.visitedProjects.add(absoluteConfig);
  // Counted here rather than at the call site so a project reached through a
  // solution config's references counts as the project it is.
  totals.projects += 1;
  const configFile = ts.readConfigFile(absoluteConfig, ts.sys.readFile);
  if (configFile.error !== undefined) {
    emitter.write({
      kind: 'diagnostic',
      category: 'project_error',
      file: projectPath,
      message: ts.flattenDiagnosticMessageText(configFile.error.messageText, ' '),
    });
    return;
  }
  const parsed = ts.parseJsonConfigFileContent(
    configFile.config,
    ts.sys,
    path.dirname(absoluteConfig),
    undefined,
    absoluteConfig
  );
  for (const error of parsed.errors || []) {
    // A config that names no inputs is a setup fact, not a hard failure: the
    // project is skipped and the caller is told why.
    emitter.write({
      kind: 'diagnostic',
      category: 'project_error',
      file: projectPath,
      message: ts.flattenDiagnosticMessageText(error.messageText, ' '),
    });
  }
  if (!parsed.fileNames || parsed.fileNames.length === 0) {
    // A solution-style config — `"files": []` with `"references"` — declares no
    // inputs of its own and delegates every file to the projects it references.
    // It is what `npm create vite`, Angular and every project-references
    // monorepo put at the repository root, and it is what the nearest-tsconfig
    // walk finds, so not following the references would leave those
    // repositories with no typed edges and nothing to read about why.
    const referenced = referencedProjects(root, absoluteConfig, parsed);
    if (referenced.length === 0) {
      emitter.write({
        kind: 'diagnostic',
        category: 'unsupported',
        file: projectPath,
        message:
          'project declares no input files and references no other project, so it can ' +
          'contribute no type-directed call edges',
      });
      return;
    }
    for (const reference of referenced) {
      emitProject({
        ts,
        emitter,
        root,
        scope,
        projectPath: reference,
        timer,
        totals,
        session,
      });
    }
    return;
  }

  // Building a program is the expensive part, and a project that does not list
  // any of the scan's files among its own inputs cannot produce a row this scan
  // keeps. Skipping it before `createProgram` is what stops a narrow scan of a
  // monorepo from type-checking every package.
  //
  // A file can still reach a program as a transitive import of a listed input,
  // and skipping here gives up that file's typed edges. That is why the skip is
  // a reported diagnostic rather than a silent one: the project's own inputs
  // are the set it owns, and a file outside them is a file the nearest-tsconfig
  // walk assigned to this project only for want of a closer one.
  if (scope !== null && !ownsAnyScopedFile(root, parsed.fileNames, scope)) {
    emitter.write({
      kind: 'diagnostic',
      category: 'unsupported',
      file: projectPath,
      message:
        'project skipped: none of the scanned files is one of its inputs, so it can ' +
        'contribute no type-directed call edges',
    });
    return;
  }

  const program = ts.createProgram({
    rootNames: parsed.fileNames,
    options: parsed.options,
  });
  const createElapsed = timer.closeStage();
  emitter.write({
    kind: 'phase',
    phase: 'create_program',
    elapsed_ms: createElapsed,
    projects: totals.projects,
    files: program.getSourceFiles().length,
    rows_emitted: emitter.rowsEmitted,
    peak_heap_bytes: heapBytes(),
  });

  const checker = program.getTypeChecker();
  const project = new ProjectEmitter({
    ts,
    emitter,
    root,
    scope,
    project: projectPath,
    emittedCallables: session.emittedCallables,
  });

  emitter.writeCounted({
    kind: 'project',
    project: projectPath,
    options_digest: compilerOptionsDigest(parsed.options),
    typescript_version: ts.version,
    file_count: program.getSourceFiles().filter((file) => !file.isDeclarationFile).length,
    stable_key: stableKey(['project', projectPath]),
  });

  /** @type {{sourceFile: any, relative: string}[]} */
  const inScopeFiles = [];
  for (const sourceFile of program.getSourceFiles()) {
    if (sourceFile.isDeclarationFile) {
      continue;
    }
    if (!isOwnedByScan(root, sourceFile.fileName)) {
      continue;
    }
    const relative = relativePath(root, sourceFile.fileName);
    if (scope !== null && !scope.has(relative)) {
      continue;
    }
    inScopeFiles.push({sourceFile, relative});
  }

  // Every in-scope file this project compiles feeds the class and
  // instantiation sets, because the dispatch expansion needs the project's
  // whole picture. Only the files no earlier project already reported are
  // emitted: overlapping projects would otherwise put the same identities on
  // the wire twice and pay the type checker twice for them.
  for (const entry of inScopeFiles) {
    collectFile({ts, checker, project, sourceFile: entry.sourceFile});
  }
  const unreportedFiles = inScopeFiles.filter(
    (entry) => !session.emittedFiles.has(entry.relative)
  );
  for (const entry of unreportedFiles) {
    session.emittedFiles.add(entry.relative);
    totals.files += 1;
    emitFile({
      ts,
      checker,
      project,
      emitter,
      sourceFile: entry.sourceFile,
      relative: entry.relative,
    });
  }

  project.flushCallables();

  emitter.write({
    kind: 'phase',
    phase: 'walk_callsites',
    elapsed_ms: timer.closeStage(),
    projects: totals.projects,
    files: totals.files,
    rows_emitted: emitter.rowsEmitted,
    peak_heap_bytes: heapBytes(),
  });
}

/**
 * Repo-relative paths of the projects a solution-style config references.
 *
 * A reference names either a config file or the directory holding one, and the
 * compiler resolves it to an absolute path. A reference that resolves outside
 * the repository is dropped: every path this sidecar reports is
 * repository-relative by contract, and a project the scan does not own can
 * contribute no row it would keep.
 *
 * @param {string} root
 * @param {string} absoluteConfig
 * @param {any} parsed
 * @returns {string[]}
 */
function referencedProjects(root, absoluteConfig, parsed) {
  const references = parsed.projectReferences || [];
  /** @type {string[]} */
  const resolved = [];
  for (const reference of references) {
    const target = reference.path;
    if (typeof target !== 'string' || target === '') {
      continue;
    }
    const absolute = path.resolve(path.dirname(absoluteConfig), target);
    let configPath = absolute;
    try {
      if (fs.statSync(absolute).isDirectory()) {
        configPath = path.join(absolute, 'tsconfig.json');
      }
    } catch (error) {
      continue;
    }
    if (!isOwnedByScan(root, configPath)) {
      continue;
    }
    resolved.push(relativePath(root, configPath));
  }
  return resolved;
}

/**
 * Whether any file the scan discovered is one of this project's own inputs.
 *
 * @param {string} root
 * @param {string[]} fileNames
 * @param {Set<string>} scope
 * @returns {boolean}
 */
function ownsAnyScopedFile(root, fileNames, scope) {
  for (const fileName of fileNames) {
    if (scope.has(relativePath(root, fileName))) {
      return true;
    }
  }
  return false;
}

/**
 * Digest of the compiler options that can change a type answer.
 *
 * Paths are excluded deliberately: an absolute output directory differs
 * between machines and must not make two otherwise identical scans miss the
 * cache.
 *
 * @param {any} options
 * @returns {string}
 */
function compilerOptionsDigest(options) {
  const semantic = [
    'strict',
    'strictNullChecks',
    'strictFunctionTypes',
    'strictBindCallApply',
    'strictPropertyInitialization',
    'noImplicitAny',
    'noImplicitThis',
    'useUnknownInCatchVariables',
    'exactOptionalPropertyTypes',
    'target',
    'module',
    'moduleResolution',
    'jsx',
    'allowJs',
    'checkJs',
    'esModuleInterop',
    'allowSyntheticDefaultImports',
    'isolatedModules',
    'verbatimModuleSyntax',
    'lib',
  ];
  const parts = [];
  for (const key of semantic) {
    const value = options[key];
    if (value === undefined) {
      continue;
    }
    parts.push(`${key}=${Array.isArray(value) ? value.join('|') : String(value)}`);
  }
  parts.sort();
  return stableKey(parts);
}

/**
 * First pass over one file: record class declarations and the classes this
 * scan instantiates.
 *
 * Emission needs the whole project's class and instantiation sets before it
 * can expand an interface-typed call into the implementations that can
 * actually receive it, so the walk happens twice: once to collect, once to
 * emit.
 *
 * @param {{ts: any, checker: any, project: ProjectEmitter, sourceFile: any}} options
 */
function collectFile(options) {
  const {ts, checker, project, sourceFile} = options;

  /** @param {any} node */
  const visit = (node) => {
    if (ts.isClassDeclaration(node) || ts.isClassExpression(node)) {
      const identity = project.callableIdentity(node, sourceFile);
      if (!project.classes.has(identity)) {
        /** @type {Map<string, any>} */
        const methods = new Map();
        for (const member of node.members || []) {
          if (!ts.isMethodDeclaration(member) && !ts.isPropertyDeclaration(member)) {
            continue;
          }
          if (member.name === undefined || !ts.isIdentifier(member.name)) {
            continue;
          }
          if (ts.isPropertyDeclaration(member)) {
            // A method-valued property (`handle = () => {}`) is dispatched
            // exactly like a declared method.
            const initializer = member.initializer;
            if (initializer === undefined || !isCallableDeclaration(ts, initializer)) {
              continue;
            }
            methods.set(String(member.name.escapedText), initializer);
            continue;
          }
          methods.set(String(member.name.escapedText), member);
        }
        project.classes.set(identity, {node, sourceFile, methods});
      }
    }
    if (ts.isNewExpression(node)) {
      const target = instantiatedClass(ts, checker, node);
      if (target !== undefined) {
        project.instantiated.add(project.callableIdentity(target.node, target.sourceFile));
      }
    }
    ts.forEachChild(node, visit);
  };
  ts.forEachChild(sourceFile, visit);
}

/**
 * The class declaration a `new` expression constructs, when the scan owns it.
 *
 * @param {any} ts
 * @param {any} checker
 * @param {any} node
 * @returns {{node: any, sourceFile: any} | undefined}
 */
function instantiatedClass(ts, checker, node) {
  let symbol;
  try {
    symbol = checker.getSymbolAtLocation(node.expression);
  } catch (error) {
    symbol = undefined;
  }
  if (symbol === undefined || symbol === null) {
    return undefined;
  }
  const resolved =
    (symbol.flags & ts.SymbolFlags.Alias) !== 0 ? tryAliasedSymbol(checker, symbol) : symbol;
  for (const declaration of (resolved && resolved.declarations) || []) {
    if (ts.isClassDeclaration(declaration) || ts.isClassExpression(declaration)) {
      return {node: declaration, sourceFile: declaration.getSourceFile()};
    }
  }
  return undefined;
}

/**
 * Implementations that can receive a call whose declared target only
 * describes it.
 *
 * Candidates are the methods of instantiated in-scope classes that carry the
 * called name and whose instance type satisfies the receiver's declared type.
 * Where the compiler exposes assignability directly that check is exact;
 * otherwise the fallback requires every member of the declared type to be
 * present on the class, which over-approximates rather than inventing a
 * target.
 *
 * @param {{ts: any, checker: any, project: ProjectEmitter, receiverType: any, methodName: string}} options
 * @returns {{node: any, sourceFile: any}[]}
 */
function dispatchImplementations(options) {
  const {ts, checker, project, receiverType, methodName} = options;
  if (methodName === '' || receiverType === undefined) {
    return [];
  }
  /** @type {{node: any, sourceFile: any}[]} */
  const implementations = [];
  for (const identity of project.instantiated) {
    const entry = project.classes.get(identity);
    if (entry === undefined) {
      continue;
    }
    const method = entry.methods.get(methodName);
    if (method === undefined) {
      continue;
    }
    if (!satisfiesDeclaredType({ts, checker, classNode: entry.node, receiverType})) {
      continue;
    }
    implementations.push({node: method, sourceFile: entry.sourceFile});
  }
  return implementations;
}

/**
 * @param {{ts: any, checker: any, classNode: any, receiverType: any}} options
 * @returns {boolean}
 */
function satisfiesDeclaredType(options) {
  const {ts, checker, classNode, receiverType} = options;
  let instanceType;
  try {
    const symbol =
      classNode.name !== undefined && classNode.name !== null
        ? checker.getSymbolAtLocation(classNode.name)
        : undefined;
    instanceType = symbol === undefined ? undefined : checker.getDeclaredTypeOfSymbol(symbol);
  } catch (error) {
    instanceType = undefined;
  }
  if (instanceType === undefined) {
    return false;
  }
  if (typeof checker.isTypeAssignableTo === 'function') {
    try {
      return checker.isTypeAssignableTo(instanceType, receiverType);
    } catch (error) {
      // Fall through to the structural check below.
    }
  }
  try {
    const required = checker.getPropertiesOfType(receiverType);
    if (required.length === 0) {
      return false;
    }
    return required.every(
      /** @param {any} property */ (property) =>
        checker.getPropertyOfType(instanceType, property.getName()) !== undefined
    );
  } catch (error) {
    return false;
  }
}

/**
 * @param {{ts: any, checker: any, project: ProjectEmitter, emitter: Emitter, sourceFile: any, relative: string}} options
 */
function emitFile(options) {
  const {ts, checker, project, emitter, sourceFile, relative} = options;
  let callsites = 0;
  let anyReceivers = 0;

  /** @param {any} node */
  const visit = (node) => {
    if (isCallableDeclaration(ts, node) || ts.isClassDeclaration(node)) {
      project.registerCallable(node, sourceFile);
    }
    if (isCallLike(ts, node)) {
      const observed = emitCallsite({ts, checker, project, emitter, sourceFile, node});
      callsites += 1;
      if (observed.anyReceiver) {
        anyReceivers += 1;
      }
    }
    ts.forEachChild(node, visit);
  };
  ts.forEachChild(sourceFile, visit);

  if (callsites > 0) {
    emitter.writeCounted({
      kind: 'any_density',
      project: project.project,
      file: relative,
      callsites,
      any_receivers: anyReceivers,
      stable_key: stableKey(['any_density', project.project, relative]),
    });
  }
}

/**
 * @param {{ts: any, checker: any, project: ProjectEmitter, emitter: Emitter, sourceFile: any, node: any}} options
 * @returns {{anyReceiver: boolean}}
 */
function emitCallsite(options) {
  const {ts, checker, project, emitter, sourceFile, node} = options;
  const relative = relativePath(project.root, sourceFile.fileName);
  const start = node.getStart(sourceFile);
  const callsiteSpan = project.span(sourceFile, start, node.end);
  // Nested calls share a start offset: `a.b().c()` and `a.b()` both begin at
  // `a`, and so do `f()()` and `f()`. An identity keyed on the start alone
  // collapses them, which drops one call site and attaches the other call's
  // targets to the survivor, so the end offset is part of the identity.
  const callsiteIdentity = `${relative}:${callsiteSpan.start_byte}:${callsiteSpan.end_byte}`;
  const callsiteKey = stableKey(['callsite', callsiteIdentity]);

  const enclosing = enclosingCallable(ts, node, sourceFile);
  const enclosingIdentity =
    enclosing === undefined ? '' : project.registerCallable(enclosing, sourceFile);

  const receiverExpression = dispatchExpression(ts, node);
  let receiverType;
  if (receiverExpression !== undefined) {
    try {
      receiverType = checker.getTypeAtLocation(receiverExpression);
    } catch (error) {
      receiverType = undefined;
    }
  }
  const isAny =
    receiverType !== undefined && (receiverType.flags & ts.TypeFlags.Any) !== 0;
  const isUnknown =
    receiverType !== undefined && (receiverType.flags & ts.TypeFlags.Unknown) !== 0;
  const unionSize =
    receiverType !== undefined && typeof receiverType.isUnion === 'function' && receiverType.isUnion()
      ? receiverType.types.length
      : 0;

  if (receiverType !== undefined) {
    let printed = '';
    try {
      printed = checker.typeToString(receiverType);
    } catch (error) {
      printed = '';
    }
    if (printed.length > PRINTED_TYPE_LIMIT) {
      printed = `${printed.slice(0, PRINTED_TYPE_LIMIT)}…`;
    }
    emitter.writeCounted({
      kind: 'receiver',
      project: project.project,
      callsite_stable_key: callsiteKey,
      printed,
      is_any: isAny,
      is_unknown: isUnknown,
      union_size: unionSize,
      stable_key: stableKey(['receiver', callsiteIdentity]),
    });
  }

  const memberName = calledMemberName(ts, node);
  /** @type {Set<string>} */
  const seen = new Set();
  let dispatchable = 0;
  let external = 0;
  let signatureOnly = 0;

  /**
   * @param {any} declaration
   * @param {string} dispatch
   */
  const emitCallee = (declaration, dispatch) => {
    const declarationFile = declaration.getSourceFile();
    const name = callableName(ts, declaration);
    const owned =
      isOwnedByScan(project.root, declarationFile.fileName) && !declarationFile.isDeclarationFile;
    if (!owned) {
      const moniker = externalMoniker(declarationFile.fileName, name);
      if (seen.has(`external:${moniker}`)) {
        return;
      }
      seen.add(`external:${moniker}`);
      external += 1;
      emitter.writeCounted({
        kind: 'callee',
        project: project.project,
        callsite_stable_key: callsiteKey,
        callable: '',
        external: moniker,
        dispatch,
        file: '',
        span: null,
        stable_key: stableKey(['callee', callsiteIdentity, moniker]),
      });
      return;
    }
    const identity = project.registerCallable(declaration, declarationFile);
    if (seen.has(identity)) {
      return;
    }
    seen.add(identity);
    if (dispatch === 'declared_signature') {
      signatureOnly += 1;
    } else {
      dispatchable += 1;
    }
    const declarationStart = declaration.getStart(declarationFile);
    emitter.writeCounted({
      kind: 'callee',
      project: project.project,
      callsite_stable_key: callsiteKey,
      callable: identity,
      external: '',
      dispatch,
      file: relativePath(project.root, declarationFile.fileName),
      span: project.span(declarationFile, declarationStart, declaration.end),
      stable_key: stableKey(['callee', callsiteIdentity, identity]),
    });
  };

  let expandDispatch = false;
  for (const declaration of calleeDeclarations(ts, checker, node)) {
    if (isTypeLevelDeclaration(ts, declaration)) {
      expandDispatch = true;
      emitCallee(declaration, 'declared_signature');
      continue;
    }
    emitCallee(declaration, 'declared');
  }

  if (expandDispatch && !isAny && !isUnknown) {
    for (const implementation of dispatchImplementations({
      ts,
      checker,
      project,
      receiverType,
      methodName: memberName,
    })) {
      emitCallee(implementation.node, 'implementation');
    }
  }

  if (unionSize > 1 && memberName !== '' && !isAny && !isUnknown) {
    for (const member of unionMemberDeclarations({ts, checker, receiverType, memberName})) {
      emitCallee(member, 'union_member');
    }
  }

  let status = 'resolved';
  let reason = '';
  if (isAny || isUnknown) {
    status = 'any_receiver';
    reason = isAny ? 'any_receiver' : 'unknown_receiver';
  } else if (dispatchable === 0 && external === 0 && signatureOnly === 0) {
    status = 'unresolved';
    reason = 'no_resolved_signature';
  } else if (dispatchable === 0 && external === 0) {
    status = 'unresolved';
    reason = 'type_level_declaration_only';
  } else if (dispatchable === 0) {
    status = 'external';
    reason = 'declaration_outside_scan';
  } else if (unionSize > 1 || dispatchable > 1) {
    status = 'union';
    reason = 'multiple_dispatch_targets';
  }

  emitter.writeCounted({
    kind: 'callsite',
    project: project.project,
    callsite: callsiteIdentity,
    enclosing: enclosingIdentity,
    file: relative,
    span: callsiteSpan,
    call_kind: callKind(ts, node),
    status,
    reason,
    stable_key: callsiteKey,
  });

  return {anyReceiver: isAny || isUnknown};
}

/**
 * The member name a call dispatches on, or the empty string for a value call.
 *
 * @param {any} ts
 * @param {any} node
 * @returns {string}
 */
function calledMemberName(ts, node) {
  const callee = node.expression;
  if (callee === undefined || callee === null) {
    return '';
  }
  if (ts.isPropertyAccessExpression(callee) && ts.isIdentifier(callee.name)) {
    return String(callee.name.escapedText);
  }
  if (ts.isElementAccessExpression(callee)) {
    const argument = callee.argumentExpression;
    if (argument !== undefined && ts.isStringLiteralLike(argument)) {
      return String(argument.text);
    }
  }
  return '';
}

/**
 * Declarations of `memberName` on each constituent of a union receiver.
 *
 * A union receiver is a set of concrete possibilities the checker already
 * knows; naming each one is more precise than falling back to the heap tier.
 *
 * @param {{ts: any, checker: any, receiverType: any, memberName: string}} options
 * @returns {any[]}
 */
function unionMemberDeclarations(options) {
  const {ts, checker, receiverType, memberName} = options;
  /** @type {any[]} */
  const declarations = [];
  if (typeof receiverType.isUnion !== 'function' || !receiverType.isUnion()) {
    return declarations;
  }
  for (const constituent of receiverType.types) {
    let property;
    try {
      property = checker.getPropertyOfType(constituent, memberName);
    } catch (error) {
      property = undefined;
    }
    for (const declaration of (property && property.declarations) || []) {
      if (isCallableDeclaration(ts, declaration) && !isTypeLevelDeclaration(ts, declaration)) {
        declarations.push(declaration);
        continue;
      }
      if (
        ts.isPropertyDeclaration(declaration) &&
        declaration.initializer !== undefined &&
        isCallableDeclaration(ts, declaration.initializer)
      ) {
        declarations.push(declaration.initializer);
      }
    }
  }
  return declarations;
}

function main() {
  let args;
  try {
    args = parseArgs(process.argv.slice(2));
  } catch (error) {
    process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
    usage();
    process.exitCode = 2;
    return;
  }
  if (!args.ndjson) {
    process.stderr.write('--ndjson is required\n');
    usage();
    process.exitCode = 2;
    return;
  }
  if (args.typescript === '') {
    process.stderr.write('--typescript is required\n');
    usage();
    process.exitCode = 2;
    return;
  }

  const root = path.resolve(args.root);
  const timer = new PhaseTimer();
  let ts;
  try {
    ts = loadTypeScript(args.typescript);
  } catch (error) {
    process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
    process.exitCode = 1;
    return;
  }

  const emitter = new Emitter(process.stdout);
  emitter.write({
    kind: 'session_begin',
    typescript_version: ts.version,
    node_version: process.version,
    typescript_path: args.typescript,
  });
  emitter.write({
    kind: 'phase',
    phase: 'resolve_typescript',
    elapsed_ms: timer.closeStage(),
    projects: 0,
    files: 0,
    rows_emitted: 0,
    peak_heap_bytes: heapBytes(),
  });

  let scope = null;
  try {
    scope = readScopeFiles(args.scopeFiles);
  } catch (error) {
    // Without the list the sidecar emits every row it always emitted and the
    // kernel drops the out-of-scope ones on receipt. That silently restores
    // the old cost, so say so rather than degrade quietly.
    emitter.write({
      kind: 'diagnostic',
      category: 'unsupported',
      file: '',
      message: `could not read --scope-files: ${
        error instanceof Error ? error.message : String(error)
      }`,
    });
  }

  const totals = {projects: 0, files: 0};
  /** @type {Session} */
  const session = {
    visitedProjects: new Set(),
    emittedFiles: new Set(),
    emittedCallables: new Set(),
  };
  if (args.projects.length === 0) {
    emitter.write({
      kind: 'diagnostic',
      category: 'setup_missing',
      file: '',
      message: 'no TypeScript project was provided; nothing to type-check',
    });
  }
  emitter.write({
    kind: 'phase',
    phase: 'discover_projects',
    elapsed_ms: timer.closeStage(),
    projects: args.projects.length,
    files: 0,
    rows_emitted: emitter.rowsEmitted,
    peak_heap_bytes: heapBytes(),
  });

  for (const projectPath of args.projects) {
    try {
      emitProject({ts, emitter, root, scope, projectPath, timer, totals, session});
    } catch (error) {
      // One unusable project must not cost the scan every other project's
      // rows, so the failure is a row and the loop continues.
      emitter.write({
        kind: 'diagnostic',
        category: 'project_error',
        file: projectPath,
        message: error instanceof Error ? error.message : String(error),
      });
    }
  }

  emitter.write({
    kind: 'session_end',
    elapsed_ms: timer.total(),
    projects: totals.projects,
    files: totals.files,
    rows_emitted: emitter.rowsEmitted,
    peak_heap_bytes: heapBytes(),
  });
  emitter.flush();
}

main();

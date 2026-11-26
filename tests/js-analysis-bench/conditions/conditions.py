#!/usr/bin/env python3
"""Condition-map corpus: generated `exports` and `imports` maps, resolved by Node.

Generates a fixed-seed corpus of packages whose `exports` or `imports` map
mixes nested condition maps, arrays, `*` patterns, `null` and invalid targets,
declaration files, bare targets, self references, maps that select nothing and
duplicate keys. Each request of each package is resolved by Node, in the
pinned sandbox image, once per condition set in `CONDITION_SETS` and in both
import and require mode. The corpus and every file Node loaded are written to
`corpus.json`, which is committed; polint's differential test
(`ts::module_graph::conditions_differential_tests`) reads it and needs no
Node.

    python3 tests/js-analysis-bench/conditions/conditions.py record   # needs docker
    python3 tests/js-analysis-bench/conditions/conditions.py check    # no docker

`check` regenerates the maps from the seed and fails unless they are the
committed ones, so the corpus cannot drift from its generator.
"""

from __future__ import annotations

import argparse
import json
import os
import posixpath
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))

from bench import load_inputs, sandbox_flags  # noqa: E402

CORPUS = HERE / "corpus.json"
SCHEMA = "polint-condition-map-corpus-1"
SEED = 0x5EED_C0DE
EXPORTS_CASES = 1400
IMPORTS_CASES = 1000
NODE_TIMEOUT_S = 600

# The conditions each Node run is started with, beside the ones Node always
# selects (`node`, `default`, `module-sync`, and `import` or `require`).
CONDITION_SETS = [
    [],
    ["worker"],
    ["browser"],
    ["module"],
    ["development"],
    ["production"],
    ["feature"],
    ["deno"],
    ["worker", "browser"],
    ["browser", "module", "development"],
    ["feature", "production"],
    ["require"],
    ["import"],
    ["polint-oracle-custom"],
]

CONDITION_KEYS = [
    "import", "require", "node", "default", "browser", "worker", "module",
    "module-sync", "development", "production", "feature", "types", "deno",
]
FILE_NAMES = ["a", "b", "c", "index", "main", "worker", "browser", "esm"]
DIRECTORIES = ["", "lib/", "dist/", "esm/", "cjs/"]
EXTENSIONS = [".js", ".mjs", ".cjs"]
PATTERN_TARGETS = ["./lib/*.js", "./lib/*", "./*.js", "./dist/*/index.js", "./esm/*.mjs", "./cjs/*.cjs"]
INVALID_TARGETS = [
    "../up.js", "a.js", "lib/a.js", "./node_modules/x.js", "/abs.js",
    "./lib/../a.js", "./lib//a.js", "./lib/./a.js",
]
DECLARATION_TARGETS = ["./index.d.ts", "./types/a.d.mts", "./lib/*.d.ts"]
EXPORTS_KEYS = [".", "./sub", "./feat/*", "./*", "./feat/x", "./lib/*.js", "./private/*"]
IMPORTS_KEYS = ["#a", "#dual", "#feat/*", "#*", "#feat/x", "#dep"]
# The text a `*` stands for in the generated requests. None is a prefix of
# another, so no file of the corpus is also a directory.
MATCHES = {"./feat/*": ["x", "y/z"], "./*": ["q"], "./lib/*.js": ["k"], "./private/*": ["p"],
           "#feat/*": ["x", "y/z"], "#*": ["q"]}

# The package every bare target of an `imports` map names.
DEP_MANIFEST = json.dumps({
    "name": "dep",
    "exports": {
        ".": {"worker": "./w.js", "browser": ["./b.js"], "import": "./i.mjs", "require": "./r.cjs", "default": "./d.js"},
        "./sub": {"development": "./sub-dev.js", "default": "./sub.js"},
        "./feat/*": "./f/*.js",
    },
})
DEP_FILES = ["w.js", "b.js", "i.mjs", "r.cjs", "d.js", "sub-dev.js", "sub.js", "f/x.js", "f/y/z.js", "f/q.js"]


class Rng:
    """mulberry32: a generator fully defined here, so the corpus does not
    depend on the Python version."""

    def __init__(self, seed: int) -> None:
        self.state = seed & 0xFFFFFFFF

    def unit(self) -> float:
        self.state = (self.state + 0x6D2B79F5) & 0xFFFFFFFF
        t = self.state
        t = ((t ^ (t >> 15)) * (t | 1)) & 0xFFFFFFFF
        t ^= (t + (((t ^ (t >> 7)) * (t | 61)) & 0xFFFFFFFF)) & 0xFFFFFFFF
        return ((t ^ (t >> 14)) & 0xFFFFFFFF) / 4294967296

    def below(self, n: int) -> int:
        return int(self.unit() * n)

    def pick(self, items: list):
        return items[self.below(len(items))]

    def chance(self, p: float) -> bool:
        return self.unit() < p


class Obj(list):
    """A JSON object as its key and value pairs, in order, duplicates kept."""


def dump(value) -> str:
    if isinstance(value, Obj):
        return "{" + ",".join(f"{json.dumps(key)}:{dump(item)}" for key, item in value) + "}"
    if isinstance(value, list):
        return "[" + ",".join(dump(item) for item in value) + "]"
    return json.dumps(value)


def plain_target(rng: Rng) -> str:
    return "./" + rng.pick(DIRECTORIES) + rng.pick(FILE_NAMES) + rng.pick(EXTENSIONS)


def target(rng: Rng, pattern: bool, imports: bool, self_name: str) -> str | None:
    roll = rng.unit()
    if roll < 0.04:
        return None
    if roll < 0.12:
        return rng.pick(INVALID_TARGETS)
    if roll < 0.16:
        return rng.pick(DECLARATION_TARGETS)
    if imports and roll < 0.30:
        bare = ["dep", "dep/sub", f"{self_name}/sub", self_name, "#a", "#dual", "node:path"]
        if pattern:
            bare += ["dep/feat/*", f"{self_name}/feat/*", "#feat/*"]
        return rng.pick(bare)
    if pattern and rng.chance(0.7):
        return rng.pick(PATTERN_TARGETS)
    return plain_target(rng)


def value(rng: Rng, depth: int, pattern: bool, imports: bool, self_name: str):
    roll = rng.unit()
    if depth >= 3 or roll < 0.42:
        return target(rng, pattern, imports, self_name)
    if roll < 0.58:
        return [value(rng, depth + 1, pattern, imports, self_name) for _ in range(rng.below(4))]
    return condition_map(rng, depth, pattern, imports, self_name)


def condition_map(rng: Rng, depth: int, pattern: bool, imports: bool, self_name: str) -> Obj:
    keys: list[str] = []
    for _ in range(1 + rng.below(4)):
        key = rng.pick(CONDITION_KEYS)
        if key not in keys:
            keys.append(key)
    if "default" not in keys and rng.chance(0.45):
        keys.append("default")
    if rng.chance(0.01):
        keys.insert(rng.below(len(keys) + 1), str(rng.below(3)))
    entries = Obj((key, value(rng, depth + 1, pattern, imports, self_name)) for key in keys)
    if rng.chance(0.03):
        key, _ = entries[rng.below(len(entries))]
        entries.append((key, value(rng, depth + 1, pattern, imports, self_name)))
    return entries


def string_targets(tree) -> list[str]:
    if isinstance(tree, str):
        return [tree]
    if isinstance(tree, Obj):
        return [found for _, item in tree for found in string_targets(item)]
    if isinstance(tree, list):
        return [found for item in tree for found in string_targets(item)]
    return []


def subpath_requests(keys: list[str], prefix: str) -> list[str]:
    """The requests a map with `keys` is asked, with `prefix` for its `.`."""
    requests: list[str] = []
    for key in keys:
        if key in MATCHES:
            base, trailer = key.split("*")
            requests += [base + match + trailer for match in MATCHES[key]]
        else:
            requests.append(key)
    if prefix.startswith("#"):
        return sorted(set(requests))
    return sorted({prefix + key[1:] for key in requests})


def exports_case(rng: Rng, index: int) -> dict:
    name = f"c{index:04d}"
    directory = f"node_modules/{name}"
    roll = rng.unit()
    if roll < 0.25:
        exports = value(rng, 1, False, False, name)
        keys = ["."]
    else:
        keys = []
        for _ in range(1 + rng.below(4)):
            key = rng.pick(EXPORTS_KEYS)
            if key not in keys:
                keys.append(key)
        exports = Obj((key, value(rng, 0, "*" in key, False, name)) for key in keys)
        if rng.chance(0.02):
            exports.append(("browser", plain_target(rng)))
    manifest = Obj([("name", name), ("exports", exports)])
    requests = subpath_requests(keys, name)
    if rng.chance(0.2):
        requests.append(f"{name}/nope")
    importers = [("main.mjs", "import"), ("main.cjs", "require")]
    return {
        "id": name,
        "kind": "exports",
        "directory": directory,
        "manifest": dump(manifest),
        "files": materialised_files(rng, directory, exports),
        "requests": [[importer, request, mode] for request in requests for importer, mode in importers],
    }


def imports_case(rng: Rng, index: int) -> dict:
    name = f"s{index:04d}"
    directory = f"pkgs/{name}"
    keys: list[str] = []
    for _ in range(1 + rng.below(4)):
        key = rng.pick(IMPORTS_KEYS)
        if key not in keys:
            keys.append(key)
    imports = Obj((key, value(rng, 0, "*" in key, True, name)) for key in keys)
    fields = [("name", name), ("imports", imports)]
    exports = None
    if rng.chance(0.5):
        exports = Obj([
            ("./sub", value(rng, 1, False, False, name)),
            ("./feat/*", value(rng, 1, True, False, name)),
        ])
        fields.append(("exports", exports))
    requests = subpath_requests(keys, "#")
    if rng.chance(0.2):
        requests.append("#nope")
    importers = [(f"{directory}/main.mjs", "import"), (f"{directory}/main.cjs", "require")]
    return {
        "id": name,
        "kind": "imports",
        "directory": directory,
        "manifest": dump(Obj(fields)),
        "files": materialised_files(rng, directory, Obj([("imports", imports), ("exports", exports)])),
        "requests": [[importer, request, mode] for request in requests for importer, mode in importers],
    }


def materialised_files(rng: Rng, directory: str, tree) -> list[str]:
    """Every file a target of `tree` may name, with each `*` replaced by every
    match of the corpus, less one in fourteen, so that most targets exist and
    a missing file is exercised too."""
    matches = sorted({match for options in MATCHES.values() for match in options})
    files: set[str] = set()
    for written in string_targets(tree):
        if not written.startswith(("./", "../")):
            continue
        for match in matches if "*" in written else [""]:
            relative = posixpath.normpath(posixpath.join(directory, written.replace("*", match)))
            if not relative.startswith(".."):
                files.add(relative)
    kept = []
    for file in sorted(files):
        if not rng.chance(1 / 14):
            kept.append(file)
    return kept


def generate() -> list[dict]:
    rng = Rng(SEED)
    cases = [exports_case(rng, index) for index in range(EXPORTS_CASES)]
    cases += [imports_case(rng, index) for index in range(IMPORTS_CASES)]
    directory = "pkgs/custom-condition"
    cases.append({
        "id": "custom-condition",
        "kind": "imports",
        "directory": directory,
        "manifest": json.dumps({"imports": {
            "#changes": {"polint-oracle-custom": "./custom.cjs", "default": "./default.cjs"},
            "#blocked": {"polint-oracle-custom": None, "default": "./default.cjs"},
        }}),
        "files": [f"{directory}/{name}.cjs" for name in ("custom", "default")],
        "requests": [[f"{directory}/main.{ext}", specifier, mode]
                     for ext, mode in [("mjs", "import"), ("cjs", "require")]
                     for specifier in ["#changes", "#blocked"]],
    })
    return cases


def materialise(cases: list[dict], root: Path) -> list[list[str]]:
    """Writes the corpus under `root` and returns every request in order."""
    def write(relative: str, text: str = "") -> None:
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        if path.is_dir():
            raise SystemExit(f"{relative} is both a file and a directory of the corpus")
        path.write_text(text)

    for importer in ("main.mjs", "main.cjs"):
        write(importer)
    write("node_modules/dep/package.json", DEP_MANIFEST)
    for file in DEP_FILES:
        write(f"node_modules/dep/{file}")
    requests: list[list[str]] = []
    for case in cases:
        write(f"{case['directory']}/package.json", case["manifest"])
        for file in case["files"]:
            write(file)
        if case["kind"] == "imports":
            write(f"{case['directory']}/main.mjs")
            write(f"{case['directory']}/main.cjs")
        requests += case["requests"]
    (root / "requests.json").write_text(json.dumps(requests))
    return requests


def run_node(image: str, tree: Path, conditions: list[str]) -> dict:
    with tempfile.TemporaryDirectory(prefix="polint-conditions-out-") as scratch:
        os.chmod(scratch, 0o777)
        argv = [
            *sandbox_flags(tmp_size="64m"),
            "-e", "CONDITIONS_ROOT=/w",
            "-e", "CONDITIONS_OUT=/out/picks.json",
            "-v", f"{tree}:/w:ro",
            "-v", f"{HERE}:/bench/conditions:ro",
            "-v", f"{scratch}:/out",
            "-w", "/w",
            image,
            "node", "--experimental-import-meta-resolve", "--no-warnings",
            *[flag for condition in conditions for flag in ("-C", condition)],
            "/bench/conditions/resolve.mjs",
        ]
        completed = subprocess.run(argv, capture_output=True, text=True, timeout=NODE_TIMEOUT_S)
        if completed.returncode != 0:
            raise SystemExit(f"node exited {completed.returncode} under {conditions}:\n{completed.stderr}")
        return json.loads((Path(scratch) / "picks.json").read_text())


def record(_args: argparse.Namespace) -> int:
    inputs = load_inputs()
    cases = generate()
    with tempfile.TemporaryDirectory(prefix="polint-conditions-tree-") as tree:
        tree_path = Path(tree)
        requests = materialise(cases, tree_path)
        os.chmod(tree, 0o755)
        runs = []
        for conditions in CONDITION_SETS:
            run = run_node(inputs["sandbox"]["image"], tree_path, conditions)
            if run["node"] != inputs["sandbox"]["node_version"]:
                raise SystemExit(f"sandbox ran node {run['node']}, expected {inputs['sandbox']['node_version']}")
            if len(run["picks"]) != len(requests):
                raise SystemExit(f"node answered {len(run['picks'])} of {len(requests)} requests")
            runs.append(run["picks"])
    position = 0
    for case in cases:
        answered = []
        for importer, specifier, mode in case["requests"]:
            loads: dict[str, list[int]] = {}
            for set_index, picks in enumerate(runs):
                if picks[position] is not None:
                    loads.setdefault(picks[position], []).append(set_index)
            answered.append({"importer": importer, "specifier": specifier, "mode": mode, "loads": loads})
            position += 1
        case["requests"] = answered
    write_corpus(inputs["sandbox"]["node_version"], cases)
    print(f"wrote {CORPUS.relative_to(HERE.parent.parent.parent)}: {len(cases)} maps, {position} requests")
    return 0


def write_corpus(node: str, cases: list[dict]) -> None:
    lines = [json.dumps(case, separators=(",", ":")) for case in cases]
    header = {
        "schema": SCHEMA,
        "seed": SEED,
        "node": node,
        "condition_sets": CONDITION_SETS,
        "importers": ["main.mjs", "main.cjs"],
        "shared": {"node_modules/dep/package.json": DEP_MANIFEST, **{f"node_modules/dep/{file}": "" for file in DEP_FILES}},
    }
    CORPUS.write_text(
        '{"header":' + json.dumps(header, separators=(",", ":")) + ',\n"cases":[\n' + ",\n".join(lines) + "\n]}\n"
    )


def check(_args: argparse.Namespace) -> int:
    corpus = json.loads(CORPUS.read_text())
    if corpus["header"]["schema"] != SCHEMA or corpus["header"]["seed"] != SEED:
        raise SystemExit(f"{CORPUS} has another schema or seed")
    if corpus["header"]["condition_sets"] != CONDITION_SETS:
        raise SystemExit(f"{CORPUS} was recorded under other condition sets")
    generated = generate()
    committed = corpus["cases"]
    if len(generated) != len(committed):
        raise SystemExit(f"{CORPUS} has {len(committed)} maps; the generator makes {len(generated)}")
    for made, kept in zip(generated, committed):
        requests = [[request["importer"], request["specifier"], request["mode"]] for request in kept["requests"]]
        if (made["id"], made["manifest"], made["files"], made["requests"]) != (
            kept["id"], kept["manifest"], kept["files"], requests
        ):
            raise SystemExit(f"{made['id']} differs from its generator; run `record`")
    print(f"{CORPUS.name}: {len(committed)} maps match the generator")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("record", help="generate the maps and resolve them with Node in the sandbox").set_defaults(
        run=record
    )
    commands.add_parser("check", help="check the committed corpus against its generator").set_defaults(run=check)
    args = parser.parse_args()
    return args.run(args)


if __name__ == "__main__":
    sys.exit(main())

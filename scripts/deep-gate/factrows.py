#!/usr/bin/env python3
"""Compare two `fact_rows_dump` output directories (the I1b oracle).

    factrows.py <before-dir> <after-dir> [--allow <allowlist>] [--sed <script>]
    factrows.py <dir> --summarize

`digests.py` compares provider output digests, which is tier I1a. This is tier
I1b: for every fact family, the sorted (canonical stable-key text, payload
digest) pairs, plus, for the five summary families, the plaintext parts column
and the attributes column. A workstream that changes which providers run moves
every downstream digest by construction, so the rows are the only oracle left.

Every family must be identical unless the allowlist says otherwise. The rules:

  absent            the family may lose every row on the after side (and nothing
                    else: a family that still has rows must match)
  any               the family may differ freely; the line delta is counted and
                    reported, never judged
  summary-column2   columns 1, 3 and 4 identical on every row, column 2 free.
                    This is W3 commit 0's oracle: routing the SCC closure to the
                    digest recipe rewrites that column on every summary row and
                    nothing else.
  i1b-line-rule     the key set is identical and every differing line's parts
                    column moves only as the I1b line rule permits (three shapes
                    of `build_control_effects`), with its attributes column
                    unchanged. This is W3 commit 2's `summary_control` oracle.

Column 2 of the five summary families is compared only when both sides carry W3
commit 0. Before it the SCC closure re-records those rows through the
`AnalysisHost` trait default as `summary:<SummaryId>` text, which no parts change
moves and any id reassignment does, so the column carries no information the key
does not. A side is recognised as pre-commit-0 by that very text.

`--summarize` renders one dump directory into the condensed record that is
committed under `research/strategy/plans/gate-reports/fact-rows/`: the per-family
row count and the SHA-256 of the family file, plus the five summary families'
rows in full. A dump directory is 93 files per cell, most of them empty, which is
neither reviewable nor within the delivery rule's file budget; a per-family
digest proves byte-identity just as well, and the summary rows are the ones an
I1b allowlist is read against by eye.

Exit codes:

  0  every difference is allowed
  1  a family moved that the allowlist does not permit
  2  the dumps or the arguments cannot be read
"""
from __future__ import annotations

import argparse
import hashlib
import os
import subprocess
import sys

SCHEMA = "polint-fact-rows-1"

SUMMARY_FAMILIES = (
    "SummaryControl",
    "SummaryCall",
    "SummaryMemory",
    "SummaryTito",
    "SummaryEvent",
)

# The text `analysis_neutral/host.rs` writes into the payload column before W3
# commit 0 routes the closure to the FNV recipe.
ID_ONLY_PREFIXES = ("summary:", "summary-event:")

RULES = ("absent", "any", "summary-column2", "i1b-line-rule")

REJECTED = 2
MOVED = 1


def reject(message: str) -> int:
    print(f"factrows.py: {message}", file=sys.stderr)
    return REJECTED


def read_index(directory: str) -> dict[str, str]:
    path = os.path.join(directory, INDEX_FILE)
    index: dict[str, str] = {}
    with open(path, encoding="utf-8") as handle:
        for line in handle:
            if "\t" in line:
                key, value = line.rstrip("\n").split("\t", 1)
                index[key] = value
    return index


INDEX_FILE = "index.txt"


def family_files(directory: str) -> set[str]:
    """Family labels in a dump directory. `index.txt` is the manifest, not a family."""
    return {
        name[: -len(".txt")]
        for name in os.listdir(directory)
        if name.endswith(".txt") and name != INDEX_FILE
    }


def read_family(directory: str, family: str, sed: str | None) -> list[str]:
    path = os.path.join(directory, f"{family}.txt")
    if not os.path.isfile(path):
        return []
    with open(path, encoding="utf-8") as handle:
        text = handle.read()
    if sed is not None:
        finished = subprocess.run(
            ["sed", "-f", sed], input=text, capture_output=True, text=True, check=False
        )
        if finished.returncode != 0:
            raise ValueError(
                f"{os.path.basename(sed)} failed on {family}: "
                f"{finished.stderr.strip() or finished.returncode}"
            )
        text = finished.stdout
    return [line for line in text.splitlines() if line]


def id_only(rows: dict[str, list[str]]) -> bool:
    """Whether a side predates W3 commit 0, read off its own summary rows."""
    for family in SUMMARY_FAMILIES:
        for line in rows.get(family, ()):
            columns = line.split("\t")
            if len(columns) > 1 and columns[1].startswith(ID_ONLY_PREFIXES):
                return True
    return False


def read_allowlist(path: str) -> dict[str, str]:
    """The `family` entries of a shared expected-move allowlist.

    The file also carries `digest <provider.id>` entries, which name the provider
    output digests a workstream expects to move; `gate.sh` reads those for the
    I1a step. They are validated here and ignored.
    """
    allowed: dict[str, str] = {}
    with open(path, encoding="utf-8") as handle:
        for number, line in enumerate(handle, 1):
            line = line.split("#", 1)[0].strip()
            if not line:
                continue
            fields = line.split()
            if fields[0] == "digest":
                if len(fields) != 2:
                    raise ValueError(f"{path}:{number}: expected `digest <provider.id>`")
                continue
            if len(fields) != 3 or fields[0] != "family" or fields[2] not in RULES:
                raise ValueError(
                    f"{path}:{number}: expected `family <Family> <{'|'.join(RULES)}>` "
                    "or `digest <provider.id>`"
                )
            allowed[fields[1]] = fields[2]
    return allowed


def control_parts_permitted(before: str, after: str) -> bool:
    """The I1b line rule, stated once in plan section 1.2.

    A differing `summary_control` line is permitted iff its before parts contain
    `exit:DoesNotReturn` and its after parts take one of three shapes, each
    matching a branch of `build_control_effects`:

      (1) simple removal, when another exit kind survives;
      (2) set-emptied replacement by `exit:Returns`, which the builder inserts
          when the removal would leave no exit kind and the body has operations;
      (3) `control=bottom`, when the exit set empties, the body has no operations
          and no unresolved call adds `exit:Unknown`.

    Shapes (1) and (2) are built from the before parts with everything but the
    exit kinds held fixed, so the invariance clause -- the `async:*` and
    `cleanup:*` parts never move under them -- is enforced by construction.
    Shape (3) discards the whole parts string, which is why the clause does not
    apply to it.
    """
    parts = before.split(";")
    if "exit:DoesNotReturn" not in parts:
        return False
    exits = sorted(part for part in parts if part.startswith("exit:"))
    tail = [part for part in parts if not part.startswith("exit:")]
    survivors = [part for part in exits if part != "exit:DoesNotReturn"]
    if survivors and after == ";".join(survivors + tail):
        return True
    if not survivors and after == ";".join(sorted(["exit:Returns"]) + tail):
        return True
    return after == "control=bottom"


def key_of(line: str) -> str:
    return line.split("\t", 1)[0]


def columns(line: str) -> list[str]:
    return line.split("\t")


def without_column2(rows: list[str]) -> list[str]:
    """The rows with the payload-digest column dropped."""
    return ["\t".join([c[0]] + c[2:]) for c in (columns(line) for line in rows)]


def compare_family(
    family: str,
    before: list[str],
    after: list[str],
    rule: str | None,
    compare_column2: bool,
) -> tuple[bool, str]:
    """(allowed, one report line) for one family."""
    # The auto-detected skip governs the default comparison only. A family with an
    # explicit rule takes full responsibility for its own columns -- W3 commit 0's
    # `summary-column2` is the rule for a pair that straddles the change, so
    # stripping the column there would erase the very delta it certifies.
    if family in SUMMARY_FAMILIES and not compare_column2 and rule is None:
        before_cmp, after_cmp = without_column2(before), without_column2(after)
        column2_note = " (column 2 not compared: a side predates W3 commit 0)"
    else:
        before_cmp, after_cmp = list(before), list(after)
        column2_note = ""

    if before_cmp == after_cmp:
        return True, f"{family}: {len(before)} -> {len(after)} rows, identical{column2_note}"

    moved = len(set(before_cmp) ^ set(after_cmp))
    head = f"{family}: {len(before)} -> {len(after)} rows, {moved} lines differ"

    if rule is None:
        return False, f"{head}  FAIL (not in the allowlist){column2_note}"
    if rule == "any":
        return True, f"{head}  allowed (any){column2_note}"
    if rule == "absent":
        if after:
            return False, f"{head}  FAIL (allowed absent, but {len(after)} rows remain)"
        return True, f"{head}  allowed (absent)"
    if rule == "summary-column2":
        if family not in SUMMARY_FAMILIES:
            return False, f"{head}  FAIL (summary-column2 applies to the summary families only)"
        keyed_before = {key_of(line): columns(line) for line in before}
        keyed_after = {key_of(line): columns(line) for line in after}
        if keyed_before.keys() != keyed_after.keys():
            return False, f"{head}  FAIL (the key set moved)"
        outside = [
            key
            for key, left in keyed_before.items()
            if [left[0], *left[2:]] != [keyed_after[key][0], *keyed_after[key][2:]]
        ]
        if outside:
            return False, f"{head}  FAIL ({len(outside)} rows moved outside column 2)"
        column2 = sum(
            1 for key, left in keyed_before.items() if left[1] != keyed_after[key][1]
        )
        return True, f"{head}  allowed (summary-column2; {column2} payload columns moved)"
    if rule == "i1b-line-rule":
        if family != "SummaryControl":
            return False, f"{head}  FAIL (the I1b line rule is a summary_control rule)"
        keyed_before = {key_of(line): columns(line) for line in before}
        keyed_after = {key_of(line): columns(line) for line in after}
        if keyed_before.keys() != keyed_after.keys():
            return False, f"{head}  FAIL (the key set moved)"
        permitted = forbidden = 0
        for key, left in keyed_before.items():
            right = keyed_after[key]
            if left == right:
                continue
            if len(left) < 4 or len(right) < 4 or left[3] != right[3]:
                forbidden += 1
                continue
            if control_parts_permitted(left[2], right[2]):
                permitted += 1
            else:
                forbidden += 1
        if forbidden:
            return False, f"{head}  FAIL ({forbidden} lines outside the I1b line rule)"
        return True, f"{head}  allowed (I1b line rule; {permitted} lines)"
    return False, f"{head}  FAIL (unknown rule {rule})"


def summarize(directory: str) -> str:
    """The committed condensed record of one dump directory."""
    index = read_index(directory)
    lines = [
        f"schema\t{index.get('schema', '')}",
        f"capability\t{index.get('capability', '')}",
        f"files\t{index.get('files', '')}",
        f"rows\t{index.get('rows', '')}",
        "",
        "# family\trows\tsha256(family file)",
    ]
    for family in sorted(family_files(directory)):
        path = os.path.join(directory, f"{family}.txt")
        with open(path, "rb") as handle:
            body = handle.read()
        rows = body.count(b"\n")
        lines.append(f"{family}\t{rows}\t{hashlib.sha256(body).hexdigest()}")
    for family in SUMMARY_FAMILIES:
        lines.append("")
        lines.append(f"# {family} rows: key\tpayload\tparts\tattributes")
        lines.extend(read_family(directory, family, None))
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(add_help=True)
    parser.error = lambda message: sys.exit(reject(message))  # type: ignore[method-assign]
    parser.add_argument("before")
    parser.add_argument("after", nargs="?")
    parser.add_argument(
        "--summarize",
        action="store_true",
        help="render one dump directory into the committed condensed record",
    )
    parser.add_argument("--allow", default=None, help="the expected-move allowlist")
    parser.add_argument(
        "--sed",
        default=None,
        help="a key normalisation applied to both sides (W3 commit 4's normalize_r9_keys.sed)",
    )
    args = parser.parse_args()

    if args.summarize:
        try:
            sys.stdout.write(summarize(args.before))
        except OSError as error:
            return reject(str(error))
        return 0
    if args.after is None:
        return reject("two directories are required unless --summarize is given")

    try:
        before_index = read_index(args.before)
        after_index = read_index(args.after)
    except OSError as error:
        return reject(f"cannot read a dump index: {error}")
    for side, index in (("before", before_index), ("after", after_index)):
        if index.get("schema") != SCHEMA:
            return reject(
                f"the {side} dump is schema {index.get('schema')!r}, expected {SCHEMA!r}"
            )

    try:
        allowed = read_allowlist(args.allow) if args.allow else {}
    except (OSError, ValueError) as error:
        return reject(str(error))

    families = sorted(family_files(args.before) | family_files(args.after))
    if not families:
        return reject("neither directory holds a family file")

    try:
        before_rows = {family: read_family(args.before, family, args.sed) for family in families}
        after_rows = {family: read_family(args.after, family, args.sed) for family in families}
    except (OSError, ValueError) as error:
        return reject(str(error))
    compare_column2 = not (id_only(before_rows) or id_only(after_rows))

    print(f"schema: {SCHEMA}  families: {len(families)}")
    print(f"summary column 2 compared: {'yes' if compare_column2 else 'no'}")
    if args.sed:
        print(f"normalisation: {os.path.basename(args.sed)}")

    failures = 0
    changed = 0
    for family in families:
        ok, line = compare_family(
            family,
            before_rows[family],
            after_rows[family],
            allowed.get(family),
            compare_column2,
        )
        if "identical" not in line:
            changed += 1
            print(line)
        if not ok:
            failures += 1
    identical = len(families) - changed
    print(f"{identical}/{len(families)} fact families identical")
    if failures:
        print(f"{failures} families moved outside the allowlist", file=sys.stderr)
        return MOVED
    return 0


if __name__ == "__main__":
    sys.exit(main())

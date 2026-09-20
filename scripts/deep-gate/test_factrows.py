#!/usr/bin/env python3
"""The allowlist and the I1b line rule of factrows.py.

    python3 scripts/deep-gate/test_factrows.py

The line rule is the part of the gate that decides whether a `summary_control`
row moved for the reason a workstream declared or for another one, so it gets
its own cases: the three permitted shapes, and the differences that look like
them but are not.
"""
from __future__ import annotations

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
FACTROWS = HERE / "factrows.py"
sys.path.insert(0, str(HERE))
import factrows  # noqa: E402

SCHEMA = "polint-fact-rows-1"
FAMILIES = ("Function", "SummaryControl", "SummaryEvent")


def write_dump(directory: Path, rows: dict[str, list[str]]) -> Path:
    directory.mkdir(parents=True, exist_ok=True)
    index = [f"schema\t{SCHEMA}", "capability\tcalls", "files\t3"]
    for family in FAMILIES:
        lines = rows.get(family, [])
        (directory / f"{family}.txt").write_text(
            "".join(line + "\n" for line in lines), encoding="utf-8"
        )
        index.append(f"{family}\t{len(lines)}")
    (directory / "index.txt").write_text("\n".join(index) + "\n", encoding="utf-8")
    return directory


def control(key: str, digest: str, parts: str, attributes: str = "present;local;native_local") -> str:
    return "\t".join([key, digest, parts, attributes])


class LineRule(unittest.TestCase):
    def test_shape_one_removal_is_permitted(self):
        before = "exit:DoesNotReturn;exit:Returns;async:Sync;cleanup:false"
        after = "exit:Returns;async:Sync;cleanup:false"
        self.assertTrue(factrows.control_parts_permitted(before, after))

    def test_shape_two_substitution_is_permitted(self):
        before = "exit:DoesNotReturn;async:Async;cleanup:true"
        after = "exit:Returns;async:Async;cleanup:true"
        self.assertTrue(factrows.control_parts_permitted(before, after))

    def test_shape_three_bottom_is_permitted(self):
        before = "exit:DoesNotReturn;async:Sync;cleanup:false"
        self.assertTrue(factrows.control_parts_permitted(before, "control=bottom"))

    def test_shape_three_discards_non_default_async_and_cleanup(self):
        # The invariance clause applies to shapes (1) and (2) only: bottom
        # replaces the whole parts string, so a non-default async or cleanup on
        # the before side goes with it.
        before = "exit:DoesNotReturn;async:Generator;cleanup:true"
        self.assertTrue(factrows.control_parts_permitted(before, "control=bottom"))

    def test_a_changed_async_part_under_shape_one_is_forbidden(self):
        before = "exit:DoesNotReturn;exit:Returns;async:Sync;cleanup:false"
        after = "exit:Returns;async:Async;cleanup:false"
        self.assertFalse(factrows.control_parts_permitted(before, after))

    def test_a_changed_cleanup_part_under_shape_two_is_forbidden(self):
        before = "exit:DoesNotReturn;async:Sync;cleanup:false"
        after = "exit:Returns;async:Sync;cleanup:true"
        self.assertFalse(factrows.control_parts_permitted(before, after))

    def test_an_added_unknown_exit_is_forbidden(self):
        before = "exit:DoesNotReturn;exit:Returns;async:Sync;cleanup:false"
        after = "exit:Returns;exit:Unknown;async:Sync;cleanup:false"
        self.assertFalse(factrows.control_parts_permitted(before, after))

    def test_a_row_without_does_not_return_may_not_move_at_all(self):
        before = "exit:Returns;async:Sync;cleanup:false"
        after = "exit:Throws;async:Sync;cleanup:false"
        self.assertFalse(factrows.control_parts_permitted(before, after))

    def test_the_exit_parts_stay_sorted_after_a_substitution(self):
        # `stable_digest_parts` sorts the exit kinds before appending async and
        # cleanup, so a substitution that produced an unsorted string is a
        # different row, not this one.
        before = "exit:DoesNotReturn;async:Sync;cleanup:false"
        self.assertFalse(
            factrows.control_parts_permitted(before, "async:Sync;cleanup:false;exit:Returns")
        )


class Allowlist(unittest.TestCase):
    def run_factrows(self, before: dict, after: dict, allow: str | None = None):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        root = Path(scratch.name)
        write_dump(root / "before", before)
        write_dump(root / "after", after)
        command = [sys.executable, str(FACTROWS), str(root / "before"), str(root / "after")]
        if allow is not None:
            (root / "allow.txt").write_text(allow, encoding="utf-8")
            command += ["--allow", str(root / "allow.txt")]
        return subprocess.run(command, capture_output=True, text=True)

    def test_identical_dumps_pass_with_no_allowlist(self):
        rows = {"Function": ["Function|file=a.go|name=f\tdeadbeef00000000"]}
        result = self.run_factrows(rows, rows)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("3/3 fact families identical", result.stdout)

    def test_an_unlisted_family_that_moves_fails(self):
        before = {"Function": ["Function|file=a.go|name=f\tdeadbeef00000000"]}
        after = {"Function": ["Function|file=a.go|name=f\tfeedface00000000"]}
        result = self.run_factrows(before, after)
        self.assertEqual(result.returncode, 1)
        self.assertIn("FAIL (not in the allowlist)", result.stdout)

    def test_column_two_of_the_summary_families_is_skipped_before_w3_commit_0(self):
        # A side whose summary payload column is `summary:<id>` text predates W3
        # commit 0; column 2 carries nothing the key does not, so it is not
        # compared and the families still read identical.
        before = {"SummaryControl": [control("SummaryControl|f=a", "summary:7", "exit:Returns;async:Sync;cleanup:false")]}
        after = {"SummaryControl": [control("SummaryControl|f=a", "1122334455667788", "exit:Returns;async:Sync;cleanup:false")]}
        result = self.run_factrows(before, after)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("summary column 2 compared: no", result.stdout)
        self.assertIn("3/3 fact families identical", result.stdout)

    def test_summary_column2_still_reads_the_column_across_w3_commit_0(self):
        # The auto-skip must not erase the delta the rule exists to certify: the
        # before side is id-only text, which is exactly the pair W3 commit 0's
        # oracle compares.
        before = {"SummaryControl": [control("SummaryControl|f=a", "summary:7", "exit:Returns;async:Sync;cleanup:false")]}
        after = {"SummaryControl": [control("SummaryControl|f=a", "1122334455667788", "exit:Returns;async:Sync;cleanup:false")]}
        result = self.run_factrows(before, after, "family SummaryControl summary-column2\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("allowed (summary-column2; 1 payload columns moved)", result.stdout)

    def test_summary_column2_allows_the_payload_column_to_move_and_nothing_else(self):
        before = {"SummaryControl": [control("SummaryControl|f=a", "aaaa000000000000", "exit:Returns;async:Sync;cleanup:false")]}
        after = {"SummaryControl": [control("SummaryControl|f=a", "bbbb000000000000", "exit:Returns;async:Sync;cleanup:false")]}
        result = self.run_factrows(before, after, "family SummaryControl summary-column2\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("allowed (summary-column2; 1 payload columns moved)", result.stdout)

        moved_parts = {"SummaryControl": [control("SummaryControl|f=a", "bbbb000000000000", "exit:Throws;async:Sync;cleanup:false")]}
        result = self.run_factrows(before, moved_parts, "family SummaryControl summary-column2\n")
        self.assertEqual(result.returncode, 1)
        self.assertIn("moved outside column 2", result.stdout)

    def test_the_i1b_line_rule_counts_permitted_lines_and_fails_others(self):
        before = {
            "SummaryControl": [
                control("SummaryControl|f=a", "aaaa000000000000", "exit:DoesNotReturn;exit:Returns;async:Sync;cleanup:false"),
                control("SummaryControl|f=b", "cccc000000000000", "exit:Returns;async:Sync;cleanup:false"),
            ]
        }
        after = {
            "SummaryControl": [
                control("SummaryControl|f=a", "bbbb000000000000", "exit:Returns;async:Sync;cleanup:false"),
                control("SummaryControl|f=b", "cccc000000000000", "exit:Returns;async:Sync;cleanup:false"),
            ]
        }
        result = self.run_factrows(before, after, "family SummaryControl i1b-line-rule\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("allowed (I1b line rule; 1 lines)", result.stdout)

        forbidden = {
            "SummaryControl": [
                control("SummaryControl|f=a", "bbbb000000000000", "exit:Returns;async:Sync;cleanup:false"),
                control("SummaryControl|f=b", "dddd000000000000", "exit:Throws;async:Sync;cleanup:false"),
            ]
        }
        result = self.run_factrows(before, forbidden, "family SummaryControl i1b-line-rule\n")
        self.assertEqual(result.returncode, 1)
        self.assertIn("outside the I1b line rule", result.stdout)

    def test_absent_requires_the_family_to_actually_empty(self):
        before = {"SummaryEvent": [control("SummaryEvent|f=a", "aaaa000000000000", "widened;loop", "control_effects;present;local")]}
        result = self.run_factrows(before, {}, "family SummaryEvent absent\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("allowed (absent)", result.stdout)

        still_there = {"SummaryEvent": [control("SummaryEvent|f=a", "bbbb000000000000", "widened;loop", "control_effects;present;local")]}
        result = self.run_factrows(before, still_there, "family SummaryEvent absent\n")
        self.assertEqual(result.returncode, 1)
        self.assertIn("rows remain", result.stdout)

    def test_a_schema_mismatch_is_rejected_rather_than_compared(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        root = Path(scratch.name)
        write_dump(root / "before", {})
        write_dump(root / "after", {})
        (root / "after" / "index.txt").write_text("schema\tpolint-fact-rows-0\n", encoding="utf-8")
        result = subprocess.run(
            [sys.executable, str(FACTROWS), str(root / "before"), str(root / "after")],
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 2)
        self.assertIn("schema", result.stderr)

    def test_summarize_renders_counts_digests_and_the_summary_rows(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        root = Path(scratch.name)
        rows = {
            "Function": ["Function|file=a.go|name=f\tdeadbeef00000000"],
            "SummaryControl": [
                control("SummaryControl|f=a", "summary:7", "exit:Returns;async:Sync;cleanup:false")
            ],
        }
        write_dump(root / "dump", rows)
        result = subprocess.run(
            [sys.executable, str(FACTROWS), str(root / "dump"), "--summarize"],
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"schema\t{SCHEMA}", result.stdout)
        # A per-family digest is the committed identity claim; an empty family is
        # the SHA-256 of nothing, which is still a claim.
        self.assertIn("Function\t1\t", result.stdout)
        self.assertIn(
            "SummaryEvent\t0\te3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            result.stdout,
        )
        self.assertIn("# SummaryControl rows: key\tpayload\tparts\tattributes", result.stdout)
        self.assertIn("SummaryControl|f=a\tsummary:7\t", result.stdout)

    def test_one_directory_without_summarize_is_a_usage_error(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        root = Path(scratch.name)
        write_dump(root / "dump", {})
        result = subprocess.run(
            [sys.executable, str(FACTROWS), str(root / "dump")], capture_output=True, text=True
        )
        self.assertEqual(result.returncode, 2)

    def test_a_normalisation_script_that_fails_is_rejected(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        root = Path(scratch.name)
        rows = {"Function": ["Function|file=a.go|name=f\tdeadbeef00000000"]}
        write_dump(root / "before", rows)
        write_dump(root / "after", rows)
        bad = root / "bad.sed"
        bad.write_text("s/unterminated\n", encoding="utf-8")
        result = subprocess.run(
            [
                sys.executable,
                str(FACTROWS),
                str(root / "before"),
                str(root / "after"),
                "--sed",
                str(bad),
            ],
            capture_output=True,
            text=True,
        )
        # A broken normalisation must not read as "the families are identical".
        self.assertEqual(result.returncode, 2)
        self.assertIn("bad.sed failed", result.stderr)

    def test_a_malformed_allowlist_is_rejected(self):
        rows = {"Function": []}
        result = self.run_factrows(rows, rows, "family SummaryControl whatever\n")
        self.assertEqual(result.returncode, 2)
        self.assertIn("expected", result.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)

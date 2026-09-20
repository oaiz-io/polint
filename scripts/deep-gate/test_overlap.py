#!/usr/bin/env python3
"""The window reading and the empty-window self-check of overlap.py.

    python3 scripts/deep-gate/test_overlap.py

The self-check is what makes an empty window a verdict rather than a vacuously
satisfied bound: a cold Go cell whose timeline holds no sidecar process either
measured nothing (exit 2) or ran nothing (exit 3), and only a sidecar cache hit
makes "no window" the honest answer.
"""
from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
OVERLAP = Path(__file__).resolve().parent / "overlap.py"

ROOT_PID = 1000
SIDECAR_PID = 1001
MB = 1 << 20

STAGE_ROW = (
    '2026-09-19T00:00:00.0Z  INFO polint::kernel::stage: stage done '
    'provider="polint.go.semantic" elapsed_ms=25000 rss_mb=100 rss_delta_mb=1 '
    'peak_rss_mb=100 facts=1 keys=1 key_mb=0 digest="abc123"\n'
)
CACHE_HIT_ROW = (
    '2026-09-19T00:00:00.0Z  INFO polint::kernel::stage: sidecar cache hit '
    'provider="polint.go.semantic"\n'
)
OTHER_ROWS = (
    '2026-09-19T00:00:00.0Z  INFO polint::kernel::stage: stage done '
    'provider="polint.go.syntax" elapsed_ms=10 rss_mb=10 rss_delta_mb=0 '
    'peak_rss_mb=10 facts=1 keys=1 key_mb=0 digest="def456"\n'
)


def polint(rss_mb: int):
    return [ROOT_PID, "polint", "polint", "/usr/local/bin/polint", rss_mb * MB, "tree"]


def sidecar(rss_mb: int, via: str = "tree", argv0: str = ".polint-go-frontend-deadbeef", exe=""):
    return [SIDECAR_PID, "polint-go-front", argv0, exe, rss_mb * MB, via]


def other(rss_mb: int):
    return [1002, "go", "/usr/bin/go", "/usr/bin/go", rss_mb * MB, "tree"]


def timeline(samples, **summary):
    base = {
        "timeline_schema": "rssrun-timeline-2",
        "enumeration": "tree+scan",
        "root_pid": ROOT_PID,
        "root_start_ticks": 123,
    }
    base.update(summary)
    return {
        "summary": base,
        "timeline": [
            {"t": index * 0.2, "total": sum(proc[4] for proc in procs), "procs": procs}
            for index, procs in enumerate(samples)
        ],
    }


class Overlap(unittest.TestCase):
    def run_overlap(self, document, stderr_text: str):
        with tempfile.TemporaryDirectory() as scratch:
            timeline_path = Path(scratch) / "timeline.json"
            timeline_path.write_text(json.dumps(document), encoding="utf-8")
            stderr_path = Path(scratch) / "probe.stderr"
            stderr_path.write_text(stderr_text, encoding="utf-8")
            return subprocess.run(
                [sys.executable, str(OVERLAP), str(timeline_path), "--stderr", str(stderr_path)],
                capture_output=True,
                text=True,
            )

    def test_the_window_is_the_first_and_last_sample_holding_a_sidecar(self):
        document = timeline(
            [
                [polint(1000)],
                [polint(2000), sidecar(6000)],
                [polint(3000), sidecar(7000), other(100)],
                [polint(9000)],
            ]
        )
        result = self.run_overlap(document, STAGE_ROW + OTHER_ROWS)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("window: 0.2 s .. 0.4 s (2 samples, 2 with a sidecar)", result.stdout)
        self.assertIn("max tree total in window MB: 10100", result.stdout)
        self.assertIn("max polint RSS in window MB: 3000", result.stdout)
        self.assertIn("max sidecar RSS in window MB: 7000", result.stdout)
        self.assertIn("max polint RSS outside window MB: 9000", result.stdout)
        self.assertIn("max tree total over run MB: 10100", result.stdout)
        self.assertIn("samples: 4", result.stdout)
        self.assertIn(f"sidecar: pid={SIDECAR_PID}", result.stdout)
        self.assertIn("rule=argv0", result.stdout)
        self.assertIn("other: go (1 samples)", result.stdout)

    def test_a_sidecar_is_recognized_by_its_materialized_directory(self):
        exe = "/home/user/.cache/polint/go-sidecars/semantic/0.3.10/abcdef/frontend"
        document = timeline(
            [[polint(1000), sidecar(500, via="scan", argv0="frontend", exe=exe)]]
        )
        result = self.run_overlap(document, STAGE_ROW)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("rule=exe", result.stdout)
        self.assertIn("via=scan", result.stdout)

    def test_a_stage_row_without_a_sidecar_process_fails_the_cell(self):
        result = self.run_overlap(timeline([[polint(1000)]]), STAGE_ROW + OTHER_ROWS)
        self.assertEqual(result.returncode, 2)
        self.assertIn("window: none", result.stdout)
        self.assertIn("sidecar ran but no sidecar process was classified", result.stderr)

    def test_a_cache_hit_makes_an_empty_window_the_honest_answer(self):
        result = self.run_overlap(
            timeline([[polint(1000)], [polint(4000)]]), CACHE_HIT_ROW + STAGE_ROW
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("window: none (sidecar cache hit)", result.stdout)
        self.assertIn("max tree total in window MB: n/a", result.stdout)
        self.assertIn("max polint RSS in window MB: n/a", result.stdout)
        self.assertIn("max sidecar RSS in window MB: n/a", result.stdout)
        self.assertIn("max polint RSS outside window MB: 4000", result.stdout)

    def test_no_stage_row_at_all_means_the_provider_never_ran(self):
        result = self.run_overlap(timeline([[polint(1000)]]), OTHER_ROWS)
        self.assertEqual(result.returncode, 3)
        self.assertIn("the provider never ran", result.stderr)

    def test_the_pre_commit_schema_is_rejected(self):
        with tempfile.TemporaryDirectory() as scratch:
            path = Path(scratch) / "old.json"
            path.write_text(
                json.dumps({"summary": {"label": "x", "peak_rss_bytes": 1}, "timeline": [[0.2, 1]]}),
                encoding="utf-8",
            )
            stderr_path = Path(scratch) / "probe.stderr"
            stderr_path.write_text(STAGE_ROW, encoding="utf-8")
            result = subprocess.run(
                [sys.executable, str(OVERLAP), str(path), "--stderr", str(stderr_path)],
                capture_output=True,
                text=True,
            )
        self.assertEqual(result.returncode, 4)
        self.assertIn("no timeline_schema", result.stderr)

    def test_a_timeline_that_names_another_enumeration_is_rejected(self):
        document = timeline([[polint(1000)]], enumeration="tree")
        result = self.run_overlap(document, STAGE_ROW)
        self.assertEqual(result.returncode, 4)
        self.assertIn("not enumerated", result.stderr)

    def test_a_usage_error_is_not_a_self_check_verdict(self):
        result = subprocess.run(
            [sys.executable, str(OVERLAP)], capture_output=True, text=True
        )
        self.assertEqual(result.returncode, 4, "argparse's own exit 2 would read as a verdict")


if __name__ == "__main__":
    unittest.main(verbosity=2)

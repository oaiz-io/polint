#!/usr/bin/env python3
"""The hygiene allowlist of report.py.

    python3 scripts/deep-gate/test_report.py

A gate report is the only thing a gate run puts in the repository, and the run
it folds is a debug-level trace over a consumer tree. The property under test is
therefore not that the report is pretty: it is that no line the allowlist does
not recognise can reach it.
"""
from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPORT = Path(__file__).resolve().parent / "report.py"

STAGE_ROWS = (
    '2026-09-19T00:00:00.0Z  INFO polint::kernel::stage: stage done '
    'provider="polint.go.syntax" elapsed_ms=120 rss_mb=300 rss_delta_mb=20 '
    'peak_rss_mb=310 facts=900 keys=900 key_mb=4 digest="aa11"\n'
    '2026-09-19T00:00:00.0Z  INFO polint::kernel::stage: stage done '
    'provider="polint.source" elapsed_ms=10 rss_mb=100 rss_delta_mb=1 '
    'peak_rss_mb=100 facts=10 keys=10 key_mb=0 digest="-"\n'
)
SAMPLER = json.dumps(
    {
        "label": "smoke",
        "cmd": ["polint", "unknowns", "--cap", "calls", "src"],
        "exit_code": 0,
        "wall_s": 12.5,
        "peak_rss_bytes": 2 * (1 << 30),
        "peak_rss_gb": 2.0,
        "samples": 60,
    }
)
LEAK = (
    "2026-09-19T00:00:00.0Z DEBUG polint::analysis: resolved import "
    "/home/someone/customer-repo/internal/billing/charge.go -> stripe\n"
)


class Report(unittest.TestCase):
    def run_report(self, stderr_text: str, *extra: str):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        directory = Path(scratch.name)
        (directory / "smoke.stderr").write_text(stderr_text, encoding="utf-8")
        (directory / "smoke.probe").write_text("exit=0\n", encoding="utf-8")
        (directory / "smoke.verdict").write_text(
            "providers: 21\n"
            "digest: 21/21 provider output digests identical\n"
            "factrows: 93/93 fact families identical\n"
            "verdict: pass\n",
            encoding="utf-8",
        )
        (directory / "smoke.timeline.json").write_text(
            json.dumps({"summary": json.loads(SAMPLER), "timeline": []}), encoding="utf-8"
        )
        return subprocess.run(
            [
                sys.executable,
                str(REPORT),
                str(directory),
                "--host",
                "16-core-box",
                "--polint",
                "0.3.10 at abcdef0",
                "--date",
                "2026-09-19",
                *extra,
            ],
            capture_output=True,
            text=True,
        )

    def test_a_clean_run_folds_into_the_section_6_shape(self):
        result = self.run_report(SAMPLER + "\n" + STAGE_ROWS, "--strict")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("# Deep-capability gate run: 2026-09-19, 16-core-box", result.stdout)
        self.assertIn("## Stage rows (smoke)", result.stdout)
        self.assertIn("| polint.go.syntax | 120 | 300 | 20 | 310 | 900 | 900 | 4 |", result.stdout)
        self.assertIn("## Gate verdicts", result.stdout)
        # The tree peak comes from the sampler summary, the polint peak from the
        # last stage row, and the capability off the recorded command line.
        self.assertIn("| smoke | calls | 0 | 12.5 | 2048 | 310 | 2 |", result.stdout)
        # The oracle verdicts come from gate.sh's verdict file, so a report can
        # be regenerated from a run directory without re-reading the transcript.
        self.assertIn("21/21 provider output digests identical", result.stdout)
        self.assertIn("93/93 fact families identical", result.stdout)
        self.assertIn("| smoke | pass |", result.stdout)

    def test_a_non_stage_line_is_rejected_under_strict(self):
        result = self.run_report(SAMPLER + "\n" + STAGE_ROWS + LEAK, "--strict")
        self.assertEqual(result.returncode, 3)
        self.assertIn("are not stage rows", result.stderr)
        self.assertNotIn("customer-repo", result.stdout)

    def test_a_non_stage_line_never_reaches_the_report(self):
        result = self.run_report(SAMPLER + "\n" + STAGE_ROWS + LEAK)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("customer-repo", result.stdout)
        self.assertNotIn("charge.go", result.stdout)
        self.assertNotIn("stripe", result.stdout)

    def test_a_resource_budget_diagnostic_is_counted_not_quoted(self):
        budget = (
            "2026-09-19T00:00:00.0Z  WARN polint: polint/resource-budget: "
            "memory ceiling 8192 MB crossed after `polint.semantic_mir` "
            "while lowering /home/someone/customer-repo/a.go\n"
        )
        result = self.run_report(SAMPLER + "\n" + STAGE_ROWS + budget)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Resource-budget diagnostics (polint/resource-budget): 1", result.stdout)
        self.assertNotIn("customer-repo", result.stdout)

    def test_a_digest_dash_is_a_value_and_not_a_failure(self):
        # polint.source, polint.ts.syntax and polint.metrics return
        # `output_digest: None` on a successful run; the kernel prints `digest="-"`.
        result = self.run_report(SAMPLER + "\n" + STAGE_ROWS, "--strict")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("| polint.source | 10 |", result.stdout)

    def test_a_missing_directory_is_a_usage_error(self):
        result = subprocess.run(
            [sys.executable, str(REPORT), "/nonexistent", "--host", "h", "--polint", "p"],
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 2)


if __name__ == "__main__":
    unittest.main(verbosity=2)

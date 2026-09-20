#!/usr/bin/env python3
"""The two enumerations of .scale-envelope/rssrun.py, run for real.

    python3 scripts/deep-gate/test_rssrun.py

Both cases exist because the sampler used to walk `/proc/<pid>/task/<pid>/children`
only: a child forked from any other thread, and a child that has reparented away
from the tree altogether, were both invisible. The Go semantic sidecar is spawned
from polint's `polint-go-semantic-prefetch` thread and can outlive the
intermediate that started it, so it was both at once.
"""
from __future__ import annotations

import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
RSSRUN = REPO / ".scale-envelope" / "rssrun.py"

# A child spawned from a thread that is not the main one. Nothing here is a
# sidecar, so only the tree descent can find it.
THREAD_SPAWNER = """
import subprocess, sys, threading, time
def spawn():
    child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(2.5)"])
    child.wait()
threading.Thread(target=spawn).start()
time.sleep(2.0)
"""

# A child that is setsid'd behind an intermediate which exits at once, so it
# reparents away and no descent from the root can reach it.
REPARENTER = """
import os, sys, time
executable = sys.argv[1]
pid = os.fork()
if pid == 0:
    os.setsid()
    if os.fork() == 0:
        os.execv(executable, [executable, "4"])
    os._exit(0)
os.waitpid(pid, 0)
time.sleep(2.0)
"""


def run_sampler(command: list[str], timeline: Path) -> dict:
    subprocess.run(
        [sys.executable, str(RSSRUN), "--label", "test", "--timeline", str(timeline), "--"]
        + command,
        check=False,
        capture_output=True,
    )
    return json.loads(timeline.read_text(encoding="utf-8"))


def records(document: dict, root_pid: int):
    """Every per-process record of every sample that is not the root's."""
    for entry in document["timeline"]:
        for proc in entry["procs"]:
            if proc[0] != root_pid:
                yield proc


class TimelineSchema(unittest.TestCase):
    def test_a_child_forked_from_another_thread_is_found_by_the_tree_descent(self):
        with tempfile.TemporaryDirectory() as scratch:
            timeline = Path(scratch) / "timeline.json"
            document = run_sampler([sys.executable, "-c", THREAD_SPAWNER], timeline)

        summary = document["summary"]
        self.assertEqual(summary["timeline_schema"], "rssrun-timeline-2")
        self.assertEqual(summary["enumeration"], "tree+scan")
        self.assertIsInstance(summary["root_pid"], int)
        self.assertIsInstance(summary["root_start_ticks"], int)

        found = [proc for proc in records(document, summary["root_pid"])]
        self.assertTrue(found, "the thread's child is never sampled")
        self.assertTrue(
            all(proc[5] == "tree" for proc in found),
            f"a non-sidecar child can only come from the descent: {found[:3]}",
        )
        self.assertTrue(
            all(len(proc) == 6 and proc[4] > 0 for proc in found),
            "every record is [pid, comm, argv0, exe, rss, via]",
        )

    def test_a_sidecar_that_reparented_away_is_found_by_the_scan(self):
        with tempfile.TemporaryDirectory() as scratch:
            source = shutil.which("sleep") or sys.executable
            fake = Path(scratch) / ".polint-go-frontend-test"
            shutil.copy(source, fake)
            fake.chmod(0o755)
            timeline = Path(scratch) / "timeline.json"
            document = run_sampler(
                [sys.executable, "-c", REPARENTER, str(fake)], timeline
            )

            root_pid = document["summary"]["root_pid"]
            sidecars = [
                proc
                for proc in records(document, root_pid)
                if proc[2] == ".polint-go-frontend-test"
            ]
            for pid in {proc[0] for proc in sidecars}:
                try:
                    os.kill(pid, signal.SIGKILL)
                except OSError:
                    pass

        self.assertTrue(sidecars, "the reparented sidecar is never sampled")
        self.assertTrue(
            all(proc[5] == "scan" for proc in sidecars),
            f"a reparented process is out of every tree: {sidecars[:3]}",
        )


if __name__ == "__main__":
    unittest.main(verbosity=2)

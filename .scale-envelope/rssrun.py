#!/usr/bin/env python3
"""Run a command under an RSS sampler with a hard address-space guard.

Samples /proc/<pid>/status for VmRSS/VmHWM every INTERVAL seconds over the whole
process tree, records a per-process timeline, and reports peak RSS + wall clock.
A RLIMIT_AS guard keeps a runaway run from taking the host down; the child dies
with an allocation failure instead of an OOM kill.

Enumeration is the union of two passes, taken at every sample:

  tree  descent from the root that reads every /proc/<pid>/task/*/children file
        of every process it reaches, not only the main thread's. A child appears
        in the children file of the thread that forked it, and polint spawns the
        Go semantic sidecar from its `polint-go-semantic-prefetch` thread, so a
        main-thread-only walk never lists it.
  scan  /proc/[0-9]* restricted to the root's pid namespace and to processes
        started at or after the root, keeping those that match a sidecar rule
        (argv0 basename, after one leading `.`, starting with
        `polint-go-frontend`; or an `exe` under `go-sidecars/semantic/`). This
        is what catches a sidecar that has reparented away from the tree.

Each per-process record says which pass found it (`via`: tree, scan or both).
`total` is the sum of VmRSS over the union, so `peak_rss_bytes` on a Go cell is
not comparable to a figure produced before this schema existed.
"""
from __future__ import annotations

import argparse
import json
import os
import resource
import signal
import subprocess
import sys
import time

INTERVAL = 0.2

TIMELINE_SCHEMA = "rssrun-timeline-2"
ENUMERATION = "tree+scan"

# The two rules that recognize a Go semantic sidecar, shared with
# scripts/deep-gate/overlap.py. The default path spawns
# `.polint-go-frontend-<cache key>` from the materialized source directory, so an
# equality test against `polint-go-frontend` never matches.
SIDECAR_ARGV0_PREFIX = "polint-go-frontend"
SIDECAR_EXE_COMPONENT = "go-sidecars/semantic/"

# /proc/<pid>/stat field 22 is starttime. Fields 1 and 2 (pid and the
# parenthesized comm, which may contain spaces) are cut off by splitting the
# text after the last ')', which makes field 3 element 0 and field 22 element 19.
STARTTIME_INDEX = 19


def read_status(pid: int) -> tuple[int, int]:
    """(VmRSS, VmHWM) in bytes for one pid; (0, 0) if it is gone."""
    try:
        with open(f"/proc/{pid}/status", "rb") as handle:
            rss = hwm = 0
            for line in handle:
                if line.startswith(b"VmRSS:"):
                    rss = int(line.split()[1]) * 1024
                elif line.startswith(b"VmHWM:"):
                    hwm = int(line.split()[1]) * 1024
                if rss and hwm:
                    break
            return rss, hwm
    except (OSError, ValueError, IndexError):
        return 0, 0


def start_ticks(pid: int) -> int | None:
    """Field 22 of /proc/<pid>/stat, the process start time in clock ticks."""
    try:
        with open(f"/proc/{pid}/stat", "r", encoding="utf-8", errors="replace") as handle:
            text = handle.read()
        return int(text[text.rindex(")") + 1 :].split()[STARTTIME_INDEX])
    except (OSError, ValueError, IndexError):
        return None


def pid_namespace(pid: int) -> str | None:
    try:
        return os.readlink(f"/proc/{pid}/ns/pid")
    except OSError:
        return None


def sidecar_rule(argv0: str, exe: str) -> str | None:
    """Which sidecar rule a process matches, or None. See overlap.py."""
    name = os.path.basename(argv0)
    if name.startswith("."):
        name = name[1:]
    if name.startswith(SIDECAR_ARGV0_PREFIX):
        return "argv0"
    if SIDECAR_EXE_COMPONENT in exe:
        return "exe"
    return None


class Identities:
    """Per-pid facts that are constant for a pid's lifetime, read once."""

    def __init__(self) -> None:
        self._cache: dict[int, tuple[str, str, str, str | None, int | None, str | None]] = {}

    def of(self, pid: int) -> tuple[str, str, str, str | None, int | None, str | None]:
        """(comm, argv0 basename, exe, pid namespace, start ticks, sidecar rule)."""
        cached = self._cache.get(pid)
        if cached is not None:
            return cached
        try:
            with open(f"/proc/{pid}/comm", "rb") as handle:
                comm = handle.read().decode("utf-8", "replace").strip()
        except OSError:
            comm = ""
        try:
            with open(f"/proc/{pid}/cmdline", "rb") as handle:
                first = handle.read().split(b"\0", 1)[0]
            argv0 = os.path.basename(first.decode("utf-8", "replace"))
        except (OSError, IndexError):
            argv0 = ""
        try:
            exe = os.readlink(f"/proc/{pid}/exe")
        except OSError:
            exe = ""
        identity = (
            comm,
            argv0,
            exe,
            pid_namespace(pid),
            start_ticks(pid),
            sidecar_rule(argv0, exe),
        )
        self._cache[pid] = identity
        return identity


def descendants(root: int) -> list[int]:
    """Every process reachable from `root` through any thread's children file."""
    pids = [root]
    seen = {root}
    frontier = [root]
    while frontier:
        parent = frontier.pop()
        try:
            tasks = os.listdir(f"/proc/{parent}/task")
        except OSError:
            continue
        for task in tasks:
            try:
                with open(f"/proc/{parent}/task/{task}/children", "rb") as handle:
                    kids = [int(tok) for tok in handle.read().split()]
            except (OSError, ValueError):
                continue
            for kid in kids:
                if kid not in seen:
                    seen.add(kid)
                    pids.append(kid)
                    frontier.append(kid)
    return pids


def sidecar_scan(
    identities: Identities, namespace: str | None, started_at: int | None
) -> list[int]:
    """Sidecar processes anywhere in the root's pid namespace, tree or not."""
    found = []
    try:
        entries = os.listdir("/proc")
    except OSError:
        return found
    for entry in entries:
        if not entry.isdigit():
            continue
        pid = int(entry)
        _, _, _, pid_ns, ticks, rule = identities.of(pid)
        if rule is None:
            continue
        if namespace is not None and pid_ns != namespace:
            continue
        if started_at is not None and (ticks is None or ticks < started_at):
            continue
        found.append(pid)
    return found


def sample(root: int, identities: Identities, namespace: str | None, started_at: int | None):
    """One sample: (total bytes, per-process records over the union)."""
    tree = descendants(root)
    scanned = sidecar_scan(identities, namespace, started_at)
    via = {pid: "tree" for pid in tree}
    for pid in scanned:
        via[pid] = "both" if pid in via else "scan"

    total = 0
    procs = []
    for pid in sorted(via):
        rss, _ = read_status(pid)
        if not rss:
            continue
        comm, argv0, exe, _, _, _ = identities.of(pid)
        total += rss
        procs.append([pid, comm, argv0, exe, rss, via[pid]])
    return total, procs


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--label", default="run")
    parser.add_argument("--timeline", default=None, help="write a JSON sample timeline here")
    parser.add_argument("--as-limit-gb", type=float, default=11.0)
    parser.add_argument("--timeout", type=float, default=3600.0)
    parser.add_argument("cmd", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    cmd = args.cmd[1:] if args.cmd and args.cmd[0] == "--" else args.cmd
    if not cmd:
        print("no command", file=sys.stderr)
        return 2

    limit = int(args.as_limit_gb * (1 << 30))

    def preexec() -> None:
        resource.setrlimit(resource.RLIMIT_AS, (limit, limit))
        os.setsid()

    started = time.monotonic()
    proc = subprocess.Popen(cmd, preexec_fn=preexec)
    identities = Identities()
    namespace = pid_namespace(proc.pid)
    started_at = start_ticks(proc.pid)
    samples: list[dict] = []
    peak = 0
    try:
        while proc.poll() is None:
            total, procs = sample(proc.pid, identities, namespace, started_at)
            if total:
                now = round(time.monotonic() - started, 3)
                samples.append({"t": now, "total": total, "procs": procs})
                peak = max(peak, total)
            if time.monotonic() - started > args.timeout:
                os.killpg(proc.pid, signal.SIGKILL)
                break
            time.sleep(INTERVAL)
    except KeyboardInterrupt:
        os.killpg(proc.pid, signal.SIGKILL)
        raise
    code = proc.wait()
    wall = time.monotonic() - started

    summary = {
        "label": args.label,
        "cmd": cmd,
        "exit_code": code,
        "wall_s": round(wall, 2),
        "peak_rss_bytes": peak,
        "peak_rss_gb": round(peak / (1 << 30), 3),
        "samples": len(samples),
        "timeline_schema": TIMELINE_SCHEMA,
        "enumeration": ENUMERATION,
        "root_pid": proc.pid,
        "root_start_ticks": started_at,
    }
    print(json.dumps(summary), file=sys.stderr)
    if args.timeline:
        with open(args.timeline, "w", encoding="utf-8") as handle:
            json.dump({"summary": summary, "timeline": samples}, handle)
    return 0 if code == 0 else 1


if __name__ == "__main__":
    sys.exit(main())

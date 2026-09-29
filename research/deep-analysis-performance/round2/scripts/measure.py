#!/usr/bin/env python3
"""Cold/warm measurement driver for the polint perf suites.

One child libtest process per sample (POLINT_PERF_CHILD_COLD_ONLY=1), so the
cold/warm classification comes only from the external cache treatment. Wall time
is the whole child process elapsed time; peak RSS is the child's own
getrusage(RUSAGE_SELF).ru_maxrss as reported in the CurvePoint JSON.
"""
import argparse, json, os, re, shutil, subprocess, sys, threading, time
from pathlib import Path

BEGIN = "<<<POLINT_PERF_POINT_BEGIN>>>"
END = "<<<POLINT_PERF_POINT_END>>>"
TEST = "eval::bench::runner::tests::perf_child_measure_entry"

FIELD_RE = re.compile(r'(\w+)=("[^"]*"|[^\s]+)')
ANSI_RE = re.compile(r'\x1b\[[0-9;]*m')
INT_FIELDS = ("elapsed_ms", "rss_mb", "rss_delta_mb", "peak_rss_mb", "facts", "keys", "key_mb")


def parse_fields(line):
    """key=value pairs from a tracing fmt line, with quotes stripped."""
    fields = {}
    for key, value in FIELD_RE.findall(line):
        fields[key] = value[1:-1] if value.startswith('"') else value
    return fields


def competing_builds():
    """Names of foreign compiler processes running right now (external-compiler guard)."""
    hits = []
    for pid in os.listdir("/proc"):
        if not pid.isdigit():
            continue
        try:
            with open(f"/proc/{pid}/cmdline", "rb") as handle:
                cmd = handle.read().replace(b"\0", b" ").decode("utf-8", "replace").strip()
        except OSError:
            continue
        if not cmd:
            continue
        head = cmd.split()[0].rsplit("/", 1)[-1]
        if head in {"rustc", "cargo", "cc1", "cc1plus", "ld", "lld", "clang", "gcc"}:
            hits.append(f"{pid}:{cmd[:120]}")
    return hits


def stamp_files(repo):
    return [str(p) for p in Path(repo).rglob("polint-store-stamp.json")]


def run_sample(binary, repo, mode, cold, extra_env):
    if os.environ.get("POLINT_CACHE_STORE") is not None:
        sys.exit("POLINT_CACHE_STORE must be unset")
    before_stamps = stamp_files(repo)
    cache = Path(repo) / ".polint" / "cache"
    if cold and cache.exists():
        shutil.rmtree(cache)
    mid_stamps = stamp_files(repo)

    env = dict(os.environ)
    env.pop("POLINT_CACHE_STORE", None)
    env["POLINT_PERF_CHILD_REPO"] = str(repo)
    env["POLINT_PERF_CHILD_COLD_ONLY"] = "1"
    env["RUST_LOG"] = "polint::kernel::stage=info"
    if mode == "syn":
        env["POLINT_PERF_CHILD_CAPABILITIES"] = "file_metrics,function_metrics,complexity_metrics"
    else:
        env.pop("POLINT_PERF_CHILD_CAPABILITIES", None)
    env.update(extra_env)

    observed = []
    stop = threading.Event()

    def watch():
        while not stop.wait(2.0):
            hits = competing_builds()
            if hits:
                observed.append(hits)

    guard = threading.Thread(target=watch, daemon=True)
    load_before = os.getloadavg()
    guard.start()
    started = time.monotonic()
    proc = subprocess.run(
        [str(binary), "--exact", TEST, "--nocapture", "--test-threads=1"],
        env=env, capture_output=True, text=True,
    )
    wall = time.monotonic() - started
    stop.set()
    guard.join(timeout=3)
    load_after = os.getloadavg()

    point = None
    if BEGIN in proc.stdout and END in proc.stdout:
        point = json.loads(proc.stdout.split(BEGIN, 1)[1].split(END, 1)[0].strip())

    stages, diagnostics = [], None
    for raw in proc.stderr.splitlines():
        line = ANSI_RE.sub('', raw)
        if "stage done" in line:
            row = parse_fields(line)
            for field in INT_FIELDS:
                if field in row:
                    row[field] = int(row[field])
            stages.append(row)
        elif "run diagnostics" in line and "diagnostics_digest" in line:
            row = parse_fields(line)
            diagnostics = {"count": int(row.get("diagnostics", 0)), "digest": row.get("diagnostics_digest")}

    return {
        "wall_s": wall,
        "exit": proc.returncode,
        "cold": cold,
        "mode": mode,
        "repo": str(repo),
        "binary": str(binary),
        "point": point,
        "stages": stages,
        "diagnostics": diagnostics,
        "load_before": load_before,
        "load_after": load_after,
        "competing_builds": observed,
        "stamps_before": before_stamps,
        "stamps_after_clear": mid_stamps,
        "stamps_final": stamp_files(repo),
        "stderr": ANSI_RE.sub("", proc.stderr),
    }


def retry_until_quiet(binary, repo, mode, cold, extra_env, out, kind, attempts=3):
    """Re-take a sample that overlapped foreign compilation; keep the rejects."""
    rejected = []
    for attempt in range(attempts):
        sample = run_sample(binary, repo, mode, cold, extra_env)
        if not sample["competing_builds"] or attempt == attempts - 1:
            sample["rejected_attempts"] = rejected
            return sample
        print(f"  [{kind}] discarded: foreign compilation overlapped "
              f"({len(sample['competing_builds'])} samples), retrying", flush=True)
        rejected.append({"wall_s": sample["wall_s"], "competing": len(sample["competing_builds"])})
        (out / f"rejected-{kind}-{attempt}.json").write_text(json.dumps(sample, indent=1))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin", required=True)
    parser.add_argument("--repo", required=True)
    parser.add_argument("--label", required=True)
    parser.add_argument("--mode", choices=["deep", "syn"], default="deep")
    parser.add_argument("--warm-reps", type=int, default=3)
    parser.add_argument("--no-cold", action="store_true")
    parser.add_argument("--out", default=".perf/results")
    parser.add_argument("--env", action="append", default=[])
    args = parser.parse_args()

    extra_env = dict(pair.split("=", 1) for pair in args.env)
    out = Path(args.out) / args.label
    out.mkdir(parents=True, exist_ok=True)
    suite = Path(args.repo).name
    samples = []
    if not args.no_cold:
        sample = retry_until_quiet(args.bin, args.repo, args.mode, True, extra_env, out, "cold")
        sample["kind"] = "cold"
        samples.append(sample)
        (out / f"{suite}-{args.mode}-cold.json").write_text(json.dumps(sample, indent=1))
        print(f"[cold] {suite}/{args.mode} wall={sample['wall_s']:.3f}s exit={sample['exit']} "
              f"rss={(sample['point'] or {}).get('peak_rss_bytes', 0)/2**30:.3f}GiB", flush=True)
    for rep in range(args.warm_reps):
        sample = retry_until_quiet(args.bin, args.repo, args.mode, False, extra_env, out, f"warm{rep + 1}")
        sample["kind"] = f"warm{rep + 1}"
        samples.append(sample)
        (out / f"{suite}-{args.mode}-warm{rep + 1}.json").write_text(json.dumps(sample, indent=1))
        print(f"[warm{rep + 1}] {suite}/{args.mode} wall={sample['wall_s']:.3f}s exit={sample['exit']} "
              f"rss={(sample['point'] or {}).get('peak_rss_bytes', 0)/2**30:.3f}GiB", flush=True)
    warm = [s["wall_s"] for s in samples if s["kind"].startswith("warm")]
    if warm:
        warm.sort()
        print(f"  median warm wall = {warm[len(warm) // 2]:.3f}s", flush=True)


if __name__ == "__main__":
    main()

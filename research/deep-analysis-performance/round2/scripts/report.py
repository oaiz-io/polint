#!/usr/bin/env python3
"""Emit the markdown before/after tables from the recorded samples."""
import json, statistics, sys
from pathlib import Path

LICENSE = {
    "jelly": ("b799ed4f0d68c670fe398830aaa51dd5c628cf74", "BSD-3-Clause"),
    "golang-tools": ("7743a285e3d261ca235408e013ec5c14cb5170e4", "BSD-3-Clause"),
    "excalidraw-excalidraw": ("0dbd2a39319d41fda37b2945dea0dcbd58d6a564", "MIT"),
    "gohugoio-hugo": ("3f35721fb2c75a1f7cc5a7a14400b66e73d4b06e", "Apache-2.0"),
}
SUITE_NAME = {
    "jelly": "jelly-callgraph-micro",
    "golang-tools": "go-x-tools-rta-callgraph",
    "excalidraw-excalidraw": "excalidraw-excalidraw-scale",
    "gohugoio-hugo": "gohugoio-hugo-scale",
}
ORDER = ["jelly", "golang-tools", "excalidraw-excalidraw", "gohugoio-hugo"]


def load(label):
    out = {}
    root = Path(".perf/results") / label
    for path in sorted(root.glob("*.json")):
        if path.name.startswith("rejected-"):
            continue
        suite, mode, kind = path.stem.rsplit("-", 2)
        out.setdefault((suite, mode), {})[kind] = json.loads(path.read_text())
    return out


def stats(kinds):
    warms = [kinds[k] for k in sorted(kinds) if k.startswith("warm")]
    cold = kinds.get("cold")
    rss = lambda s: (s["point"] or {}).get("peak_rss_bytes", 0) / 2 ** 30
    return {
        "cold_wall": cold["wall_s"] if cold else None,
        "cold_rss": rss(cold) if cold else None,
        "warm_wall": statistics.median(s["wall_s"] for s in warms) if warms else None,
        "warm_rss": statistics.median(rss(s) for s in warms) if warms else None,
        "warm_walls": [round(s["wall_s"], 3) for s in warms],
        "keys": warms[-1]["stages"][-1].get("keys") if warms and warms[-1]["stages"] else None,
        "key_mb": warms[-1]["stages"][-1].get("key_mb") if warms and warms[-1]["stages"] else None,
    }


def cell(before, after, ratio=False, unit=""):
    if before is None or after is None:
        return "N/A"
    if ratio:
        return f"{before:.3f} → {after:.3f}" if after else "N/A"
    return f"{before:.3f} → {after:.3f}{unit}"


def table(base, new, mode, title):
    print(f"### {title}\n")
    print("| Suite / SHA / license | Warm wall s, before → after | Speedup | "
          "Warm peak RSS GiB, before → after | RSS Δ | Cold wall s, before → after | "
          "Cold peak RSS GiB, before → after |")
    print("|---|---:|---:|---:|---:|---:|---:|")
    for suite in ORDER:
        if (suite, mode) not in base or (suite, mode) not in new:
            continue
        b, n = stats(base[(suite, mode)]), stats(new[(suite, mode)])
        speed = f"{b['warm_wall'] / n['warm_wall']:.3f}×" if b["warm_wall"] and n["warm_wall"] else "N/A"
        delta = (f"{(n['warm_rss'] - b['warm_rss']) / b['warm_rss'] * 100:+.2f}%"
                 if b["warm_rss"] else "N/A")
        sha, lic = LICENSE[suite]
        print(f"| {SUITE_NAME[suite]} / `{sha}` / {lic} | {cell(b['warm_wall'], n['warm_wall'])} | "
              f"{speed} | {cell(b['warm_rss'], n['warm_rss'])} | {delta} | "
              f"{cell(b['cold_wall'], n['cold_wall'])} | {cell(b['cold_rss'], n['cold_rss'])} |")
    print()


def repetitions(base, new, mode):
    print(f"### {mode} warm repetitions\n")
    print("| Suite | Before seconds | After seconds |")
    print("|---|---|---|")
    for suite in ORDER:
        if (suite, mode) not in base or (suite, mode) not in new:
            continue
        b, n = stats(base[(suite, mode)]), stats(new[(suite, mode)])
        print(f"| {SUITE_NAME[suite]} | {', '.join(map(str, b['warm_walls']))} | "
              f"{', '.join(map(str, n['warm_walls']))} |")
    print()


def identity(base, new):
    print("### Interned identity retained\n")
    print("| Suite / mode | Keys | Key text MiB, before → after | Reduction |")
    print("|---|---:|---:|---:|")
    for suite in ORDER:
        for mode in ("deep", "syn"):
            if (suite, mode) not in base or (suite, mode) not in new:
                continue
            b, n = stats(base[(suite, mode)]), stats(new[(suite, mode)])
            if not b["key_mb"]:
                continue
            keys = f"{b['keys']:,}" if b["keys"] == n["keys"] else f"{b['keys']:,} → {n['keys']:,}"
            print(f"| {SUITE_NAME[suite]} / {mode} | {keys} | {b['key_mb']} → {n['key_mb']} | "
                  f"{(n['key_mb'] - b['key_mb']) / b['key_mb'] * 100:+.1f}% |")
    print()


def digests(base, new):
    print("### Provider and diagnostic digest equality\n")
    print("| Suite / mode / sample | Providers compared | Verdict |")
    print("|---|---:|---|")
    for suite in ORDER:
        for mode in ("deep", "syn"):
            for kind in ("cold", "warm1", "warm2", "warm3"):
                left = base.get((suite, mode), {}).get(kind)
                right = new.get((suite, mode), {}).get(kind)
                if not left or not right:
                    continue
                lmap = {s["provider"]: s.get("digest") for s in left["stages"]}
                rmap = {s["provider"]: s.get("digest") for s in right["stages"]}
                diff = {k for k in set(lmap) | set(rmap) if lmap.get(k) != rmap.get(k)}
                ldiag = (left.get("diagnostics") or {})
                rdiag = (right.get("diagnostics") or {})
                verdict = "identical" if not diff and ldiag == rdiag else f"DIFFERS: {sorted(diff)} {ldiag} vs {rdiag}"
                print(f"| {SUITE_NAME[suite]} / {mode} / {kind} | {len(lmap)} | {verdict} |")
    print()


def main():
    base, new = load(sys.argv[1]), load(sys.argv[2])
    table(base, new, "deep", "Deep workloads")
    table(base, new, "syn", "Syntactic workloads")
    repetitions(base, new, "deep")
    identity(base, new)
    digests(base, new)


if __name__ == "__main__":
    main()

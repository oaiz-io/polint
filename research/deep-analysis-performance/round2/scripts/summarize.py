#!/usr/bin/env python3
"""Per-suite cold/warm summary and digest comparison across labels."""
import json, statistics, sys
from pathlib import Path

SUITES = ["jelly", "go-x-tools-rta-callgraph", "excalidraw-excalidraw", "gohugoio-hugo", "golang-tools"]


def load(label):
    out = {}
    root = Path(".perf/results") / label
    if not root.exists():
        return out
    for path in sorted(root.glob("*.json")):
        data = json.loads(path.read_text())
        stem = path.stem
        suite, mode, kind = stem.rsplit("-", 2) if stem.count("-") >= 2 else (stem, "deep", "cold")
        out.setdefault((suite, mode), {})[kind] = data
    return out


def digests(sample):
    rows = {s["provider"]: s.get("digest", "-") for s in sample.get("stages", [])}
    rows["__diagnostics__"] = (sample.get("diagnostics") or {}).get("digest")
    rows["__diagnostic_count__"] = (sample.get("diagnostics") or {}).get("count")
    return rows


def summarize(label):
    data = load(label)
    rows = []
    for (suite, mode), kinds in sorted(data.items()):
        warms = [kinds[k] for k in sorted(kinds) if k.startswith("warm")]
        cold = kinds.get("cold")
        warm_wall = statistics.median([w["wall_s"] for w in warms]) if warms else None
        warm_rss = statistics.median([(w["point"] or {}).get("peak_rss_bytes", 0) for w in warms]) if warms else None
        rows.append({
            "suite": suite, "mode": mode,
            "cold_wall": cold["wall_s"] if cold else None,
            "cold_rss": (cold["point"] or {}).get("peak_rss_bytes") if cold else None,
            "warm_wall": warm_wall, "warm_rss": warm_rss,
            "warm_walls": [round(w["wall_s"], 3) for w in warms],
            "keys": warms[-1]["stages"][-1].get("keys") if warms and warms[-1]["stages"] else None,
            "key_mb": warms[-1]["stages"][-1].get("key_mb") if warms and warms[-1]["stages"] else None,
        })
    return rows, data


def main():
    base_label, new_label = sys.argv[1], (sys.argv[2] if len(sys.argv) > 2 else None)
    base_rows, base_data = summarize(base_label)
    if not new_label:
        for row in base_rows:
            print(f"{row['suite']:26s} {row['mode']:4s} cold {row['cold_wall'] or 0:8.3f}s "
                  f"{(row['cold_rss'] or 0)/2**30:6.3f}GiB  warm {row['warm_wall'] or 0:8.3f}s "
                  f"{(row['warm_rss'] or 0)/2**30:6.3f}GiB keys={row['keys']} key_mb={row['key_mb']} {row['warm_walls']}")
        return
    new_rows, new_data = summarize(new_label)
    index = {(r["suite"], r["mode"]): r for r in new_rows}
    print(f"{'suite/mode':32s} {'cold wall':>20s} {'cold RSS':>20s} {'warm wall':>20s} {'warm RSS':>20s}")
    for row in base_rows:
        other = index.get((row["suite"], row["mode"]))
        if not other:
            continue
        def pair(a, b, scale=1.0, unit=""):
            if a is None or b is None:
                return "n/a"
            return f"{a/scale:.3f}->{b/scale:.3f}{unit} ({a/b:.2f}x)" if b else "n/a"
        print(f"{row['suite']+'/'+row['mode']:32s} "
              f"{pair(row['cold_wall'], other['cold_wall']):>20s} "
              f"{pair(row['cold_rss'], other['cold_rss'], 2**30):>20s} "
              f"{pair(row['warm_wall'], other['warm_wall']):>20s} "
              f"{pair(row['warm_rss'], other['warm_rss'], 2**30):>20s}")
    print()
    for key, kinds in sorted(base_data.items()):
        for kind, sample in sorted(kinds.items()):
            other = new_data.get(key, {}).get(kind)
            if not other:
                continue
            left, right = digests(sample), digests(other)
            diff = {k: (left.get(k), right.get(k)) for k in set(left) | set(right) if left.get(k) != right.get(k)}
            status = "IDENTICAL" if not diff else f"DIFFERS {diff}"
            print(f"digests {key[0]}/{key[1]}/{kind}: {status}")


if __name__ == "__main__":
    main()

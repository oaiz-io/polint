#!/usr/bin/env python3
"""Split the working-tree diff into reviewable slices by hunk content."""
import re, subprocess, sys
from pathlib import Path

INSTRUMENTATION = {
    "crates/polint/src/analysis/semantic_graph/provider.rs",
    "crates/polint/src/ts/semantic_graph_build.rs",
}
STREAMING = ("compare_canonical", "sort_by_cached_key", "sort_by_key")


def hunks(diff):
    """(file_header, [hunk_text]) per file in a unified diff."""
    files, header, current = [], None, []
    for block in re.split(r"(?m)^(?=diff --git )", diff):
        if not block.strip():
            continue
        lines = block.split("\n")
        head, rest = [], []
        for index, line in enumerate(lines):
            if line.startswith("@@"):
                rest = lines[index:]
                break
            head.append(line)
        else:
            files.append(("\n".join(head), []))
            continue
        parts, buffer = [], []
        for line in rest:
            if line.startswith("@@") and buffer:
                parts.append("\n".join(buffer))
                buffer = [line]
            else:
                buffer.append(line)
        if buffer:
            parts.append("\n".join(buffer))
        files.append(("\n".join(head), parts))
    return files


def build(diff, keep):
    out = []
    for header, parts in hunks(diff):
        path = re.search(r"^\+\+\+ b/(.*)$", header, re.M)
        path = path.group(1) if path else ""
        chosen = [part for part in parts if keep(path, part)]
        if not chosen:
            continue
        out.append(header.rstrip("\n"))
        out.extend(part.rstrip("\n") for part in chosen)
    return "\n".join(out) + "\n" if out else ""


def main():
    diff = subprocess.run(["git", "diff", "-U6", "HEAD"], capture_output=True, text=True).stdout
    slice_a = build(diff, lambda path, part: path not in INSTRUMENTATION
                    and not any(token in part for token in STREAMING))
    slice_b = build(diff, lambda path, part: path not in INSTRUMENTATION
                    and any(token in part for token in STREAMING))
    slice_c = build(diff, lambda path, part: path in INSTRUMENTATION)
    for name, text in (("a", slice_a), ("b", slice_b), ("c", slice_c)):
        Path(f"/tmp/slice-{name}.patch").write_text(text)
        print(f"slice-{name}: {len(text.splitlines())} lines, "
              f"{sum(1 for line in text.splitlines() if line.startswith('@@'))} hunks")


if __name__ == "__main__":
    main()

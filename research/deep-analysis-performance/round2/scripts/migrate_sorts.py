#!/usr/bin/env python3
"""Rewrite stable-key sort comparators to stream instead of materialize.

`(interner.resolve(l.k), l.id).cmp(&(interner.resolve(r.k), r.id))` becomes
`interner.compare_canonical(l.k, r.k).then_with(|| l.id.cmp(&r.id))`, which is
the same ordering without expanding either key.
"""
import re, sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
from migrate_keys import match_delims, split_top

RESOLVE = re.compile(r"^(?P<recv>[\w.]*interner)\s*\.\s*resolve\(\s*(?P<id>.*?)\s*\)$", re.S)


def element(text):
    text = text.strip()
    found = RESOLVE.match(text)
    return ("key", found.group("recv"), found.group("id")) if found else ("plain", None, text)


def rewrite(source):
    out, index, changed = [], 0, 0
    for found in list(re.finditer(r"\)\s*\.cmp\(\s*&", source)):
        pass
    while True:
        found = re.search(r"\.cmp\(\s*&\s*\(", source[index:])
        if not found:
            out.append(source[index:])
            break
        cmp_at = index + found.start()
        # Left tuple: the parenthesised expression ending just before `.cmp(`.
        left_end = cmp_at
        while left_end > index and source[left_end - 1].isspace():
            left_end -= 1
        if left_end == index or source[left_end - 1] != ")":
            out.append(source[index:cmp_at + 5])
            index = cmp_at + 5
            continue
        depth, left_start = 0, None
        for pos in range(left_end - 1, index - 1, -1):
            if source[pos] == ")":
                depth += 1
            elif source[pos] == "(":
                depth -= 1
                if depth == 0:
                    left_start = pos
                    break
        if left_start is None:
            out.append(source[index:cmp_at + 5])
            index = cmp_at + 5
            continue
        right_open = index + found.end() - 1
        try:
            right_close = match_delims(source, right_open, "(", ")")
            call_close = match_delims(source, cmp_at + len(".cmp"), "(", ")")
        except ValueError:
            out.append(source[index:cmp_at + 5])
            index = cmp_at + 5
            continue
        left_items = split_top(source[left_start + 1:left_end - 1])
        right_items = split_top(source[right_open + 1:right_close - 1])
        if len(left_items) != len(right_items) or len(left_items) < 2:
            out.append(source[index:cmp_at + 5])
            index = cmp_at + 5
            continue
        pairs = [(element(a), element(b)) for a, b in zip(left_items, right_items)]
        if not any(a[0] == "key" and b[0] == "key" for a, b in pairs):
            out.append(source[index:cmp_at + 5])
            index = cmp_at + 5
            continue
        if any((a[0] == "key") != (b[0] == "key") for a, b in pairs):
            out.append(source[index:cmp_at + 5])
            index = cmp_at + 5
            continue
        terms = []
        for (kind, recv, left_value), (_, _, right_value) in pairs:
            if kind == "key":
                terms.append(f"{recv}.compare_canonical({left_value}, {right_value})")
            else:
                terms.append(f"{left_value}.cmp(&{right_value})")
        expression = terms[0] + "".join(f".then_with(|| {term})" for term in terms[1:])
        out.append(source[index:left_start])
        out.append(expression)
        index = call_close
        changed += 1
    return "".join(out), changed


def main():
    total = 0
    for path in sys.argv[1:]:
        path = Path(path)
        source = path.read_text()
        updated, changed = rewrite(source)
        if changed:
            path.write_text(updated)
            print(f"{changed:3d} {path}")
            total += changed
    print(f"total {total}")


if __name__ == "__main__":
    main()

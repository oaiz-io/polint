#!/usr/bin/env python3
"""Rewrite composite stable-key construction sites to share child identities.

Finds `<fn>(interner, FactFamily::X, [ (label, value), ... ])` calls whose parts
include `interner.resolve(<id>).to_string()` and rewrites them to the `KeyPart`
form, so the embedded parent key is referenced instead of expanded.
"""
import re, sys
from pathlib import Path

RESOLVE = re.compile(r"^(?P<recv>[A-Za-z_][\w.]*)\s*\.\s*resolve\(\s*(?P<id>.*?)\s*\)\s*\.to_string\(\)$", re.S)
RESOLVE_AS_REF = re.compile(r"^(?P<recv>[A-Za-z_][\w.]*)\s*\.\s*resolve\(\s*(?P<id>.*?)\s*\)\s*\.as_ref\(\)\s*\.to_string\(\)$", re.S)
LITERAL = re.compile(r'^("(?:[^"\\]|\\.)*")\s*\.to_string\(\)$', re.S)
STR_CALL = re.compile(r"^(?P<call>[\w:.]*(?:_label|_str|as_str|_name)\s*\((?:[^()]|\([^()]*\))*\))\s*\.to_string\(\)$", re.S)


def match_delims(text, start, open_ch, close_ch):
    """Index just past the delimiter pair opening at `start`."""
    depth = 0
    index = start
    in_str = False
    in_char = False
    while index < len(text):
        ch = text[index]
        if in_str:
            if ch == "\\":
                index += 2
                continue
            if ch == '"':
                in_str = False
        elif in_char:
            if ch == "\\":
                index += 2
                continue
            if ch == "'":
                in_char = False
        elif ch == '"':
            in_str = True
        elif ch == "'" and index + 2 < len(text) and (text[index + 2] == "'" or text[index + 1] == "\\"):
            in_char = True
        elif ch == open_ch:
            depth += 1
        elif ch == close_ch:
            depth -= 1
            if depth == 0:
                return index + 1
        index += 1
    raise ValueError("unbalanced")


def split_top(text):
    """Split a comma-separated list at depth zero."""
    parts, depth, start, in_str = [], 0, 0, False
    index = 0
    while index < len(text):
        ch = text[index]
        if in_str:
            if ch == "\\":
                index += 2
                continue
            if ch == '"':
                in_str = False
        elif ch == '"':
            in_str = True
        elif ch in "([{":
            depth += 1
        elif ch in ")]}":
            depth -= 1
        elif ch == "," and depth == 0:
            parts.append(text[start:index])
            start = index + 1
        index += 1
    tail = text[start:]
    if tail.strip():
        parts.append(tail)
    return parts


def convert_value(value):
    value = value.strip()
    for pattern in (RESOLVE_AS_REF, RESOLVE):
        found = pattern.match(value)
        if found:
            return f"KeyPart::Key({found.group('id').strip()})", True
    found = LITERAL.match(value)
    if found:
        return f"KeyPart::Text({found.group(1)})", False
    found = STR_CALL.match(value)
    if found:
        return f"KeyPart::Text({found.group('call').strip()})", False
    return f"KeyPart::Text(&{value})", False


def rewrite(source, fn_name, new_fn):
    out = []
    index = 0
    changed = 0
    pattern = re.compile(r"(?<![\w.])" + re.escape(fn_name) + r"\(")
    while True:
        found = pattern.search(source, index)
        if not found:
            out.append(source[index:])
            break
        open_paren = found.end() - 1
        try:
            close = match_delims(source, open_paren, "(", ")")
        except ValueError:
            out.append(source[index:found.end()])
            index = found.end()
            continue
        inner = source[open_paren + 1:close - 1]
        args = split_top(inner)
        if len(args) != 3 or "FactFamily::" not in args[1]:
            out.append(source[index:found.end()])
            index = found.end()
            continue
        array = args[2].strip()
        if array.startswith("&"):
            array = array[1:].strip()
        if not array.startswith("[") or not array.endswith("]"):
            out.append(source[index:found.end()])
            index = found.end()
            continue
        elements = split_top(array[1:-1])
        converted, any_key = [], False
        ok = True
        for element in elements:
            element = element.strip()
            if not element.startswith("(") or not element.endswith(")"):
                ok = False
                break
            pair = split_top(element[1:-1])
            if len(pair) != 2:
                ok = False
                break
            label = pair[0].strip()
            value, is_key = convert_value(pair[1])
            any_key = any_key or is_key
            converted.append(f"({label}, {value})")
        if not ok or not any_key:
            out.append(source[index:found.end()])
            index = found.end()
            continue
        body = ", ".join(converted)
        out.append(source[index:found.start()])
        out.append(f"{new_fn}({args[0].strip()}, {args[1].strip()}, [{body}])")
        index = close
        changed += 1
    return "".join(out), changed


def main():
    fn_name, new_fn = sys.argv[1], sys.argv[2]
    total = 0
    for path in sys.argv[3:]:
        path = Path(path)
        source = path.read_text()
        updated, changed = rewrite(source, fn_name, new_fn)
        if changed:
            path.write_text(updated)
            print(f"{changed:3d} {path}")
            total += changed
    print(f"total {total}")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Ensure migrated files import the composite stable-key helpers."""
import re, sys
from pathlib import Path

def ensure(source, module, name):
    if re.search(r"use\s+crate::" + module + r"::\{[^}]*\b" + name + r"\b", source, re.S):
        return source, False
    if re.search(r"use\s+crate::" + module + r"::" + name + r"\s*;", source):
        return source, False
    braced = re.search(r"use\s+crate::" + module + r"::\{(?P<items>[^}]*)\}\s*;", source, re.S)
    if braced:
        items = [item.strip() for item in braced.group("items").split(",") if item.strip()]
        items.append(name)
        # Rust import order: types (capitalised) before functions, each alphabetical.
        items = sorted(set(items), key=lambda item: (item[:1].islower(), item))
        replacement = "use crate::%s::{%s};" % (module, ", ".join(items))
        return source[:braced.start()] + replacement + source[braced.end():], True
    single = re.search(r"use\s+crate::" + module + r"::(?P<item>[\w:]+)\s*;", source)
    if single:
        items = sorted({single.group("item"), name}, key=lambda item: (item[:1].islower(), item))
        replacement = "use crate::%s::{%s};" % (module, ", ".join(items))
        return source[:single.start()] + replacement + source[single.end():], True
    anchor = list(re.finditer(r"^use .*?;\n", source, re.M))
    line = "use crate::%s::%s;\n" % (module, name)
    if anchor:
        at = anchor[-1].end()
        return source[:at] + line + source[at:], True
    return line + source, True


def main():
    for path in sys.argv[1:]:
        path = Path(path)
        source = path.read_text()
        original = source
        if "stable_key_from_key_parts" in source:
            source, _ = ensure(source, "analysis_api", "stable_key_from_key_parts")
        if "KeyPart::" in source:
            source, _ = ensure(source, "internal_core", "KeyPart")
        if source != original:
            path.write_text(source)
            print(f"imports {path}")


if __name__ == "__main__":
    main()

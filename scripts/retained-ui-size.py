#!/usr/bin/env python3
"""Reproduce the retained-UI ledger; no builds and no tracked files are changed.

Usage: python3 scripts/retained-ui-size.py [BASE [CURRENT]]
CURRENT defaults to HEAD. Physical and nonblank lines include comments. Tests
are separate. Rust source is additionally compared using the same rustfmt on
both revisions (120 columns, Max small heuristics, no child-module traversal).
The cfg(test) scanner is intentionally limited to this frontend, not a Rust parser.
"""
from pathlib import Path
import difflib
import json
import os
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
BASE = sys.argv[1] if len(sys.argv) > 1 else "415aeff"
CURRENT = sys.argv[2] if len(sys.argv) > 2 else "HEAD"
SCOPE = {"app.rs", "app/attachments.rs", "app/projects.rs", "app/navigation.rs",
         "app/notices.rs", "app/ripple.rs", "app/composer_status.rs", "scroll.rs",
         "tooltip.rs", "tooltip/text.rs", "app/code_view.rs"}
RUSTFMT = os.environ.get("RUSTFMT", str(Path.home() / ".cargo/bin/rustfmt"))


def git(*args):
    return subprocess.check_output(["git", "-C", str(ROOT), *args], text=True)


def source(rev, path):
    return git("show", f"{rev}:{path}")


def code_mask(text):
    # Preserve positions while masking strings/chars/comments, including raw Rust
    # strings. Lifetime apostrophes remain code. Nested comments are not present
    # in the cfg(test) item boundaries used by this ledger.
    pattern = r'r(#+)?".*?"\1|r".*?"|"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\\n])\'|//[^\n]*|/\*.*?\*/'
    return re.sub(pattern, lambda m: re.sub(r"[^\n]", " ", m[0]), text, flags=re.S)


def item_end(mask, start):
    parens = brackets = braces = 0
    field = bool(re.match(r"(?:pub(?:\([^)]*\))?\s+)?\w+\s*:", mask[start:]))
    use = bool(re.match(r"(?:pub(?:\([^)]*\))?\s+)?use\b", mask[start:]))
    opened = False
    for i in range(start, len(mask)):
        char = mask[i]
        if char == "(": parens += 1
        elif char == ")": parens -= 1
        elif char == "[": brackets += 1
        elif char == "]": brackets -= 1
        elif char == "{":
            if parens == brackets == braces == 0: opened = True
            braces += 1
        elif char == "}":
            braces -= 1
            if opened and parens == brackets == braces == 0 and not (use or field):
                return i + 1
        elif char == ";" and parens == brackets == braces == 0:
            return i + 1
        elif char == "," and field and parens == brackets == braces == 0:
            return i + 1
    raise ValueError("Unclosed cfg(test) item: " + mask[start:start+100])


def production(text):
    # Remove each test-only item, not the entire suffix after a test module:
    # app/attachments.rs has production export handling after its inline tests.
    attr = re.compile(r"^[ \t]*#\[cfg\((?:test|all\(test,\s*not\(target_os\s*=\s*\"android\"\)\))\)\]", re.M)
    while m := attr.search(text):
        start = m.end()
        while True:
            while start < len(text) and text[start].isspace(): start += 1
            if text.startswith("#[", start):
                start = text.index("]", start) + 1
            else: break
        end = item_end(code_mask(text), start)
        if text[end:end+1] == "\n": end += 1
        text = text[:m.start()] + text[end:]
    return text


def test_only(path):
    return "/tests/" in path or path.endswith("/tests.rs") or path.endswith("_tests.rs") or path.endswith("/test_ui.rs")


def in_scope(path):
    local = path.removeprefix("frontend/src/")
    return local in SCOPE or local in {"app/ui_owner.rs", "app/mobile_input.rs", "app/projection.rs"} or local.startswith("app/ui/")


def count(text):
    lines = text.splitlines()
    return [len(lines), sum(bool(line.strip()) for line in lines)]


def plus(a, b): return [x+y for x, y in zip(a, b)]
def minus(a, b): return [x-y for x, y in zip(a, b)]


def formatted(path, text):
    if not path.endswith(".rs") or not text.strip(): return text
    return subprocess.check_output([RUSTFMT, "--edition", "2024", "--emit", "stdout", "--config",
                                    "max_width=120,use_small_heuristics=Max,skip_children=true"], input=text, text=True)


def main():
    paths = {rev: set(git("ls-tree", "-r", "--name-only", rev, "frontend").splitlines()) for rev in [BASE, CURRENT]}
    touched = set(git("diff", "--name-only", BASE, CURRENT, "--", "frontend").splitlines())
    # Changed image/font assets are not source lines. Keep textual host changes
    # (including Java) in the outside-scope charge, rather than filtering to Rust.
    binary = {line.split("\t", 2)[2] for line in git("diff", "--numstat", BASE, CURRENT, "--", "frontend").splitlines()
              if line.startswith("-\t-\t")}
    relevant = sorted(p for p in paths[BASE] | paths[CURRENT] if p not in binary and (in_scope(p) or p in touched))
    totals = {mode: {rev: {group: [0, 0] for group in ["scope", "other", "tests", "all"]} for rev in [BASE, CURRENT]}
              for mode in ["physical", "normalized"]}
    rows = []
    churn = [0, 0, 0]  # deleted, added, exactly matched production lines after normalization
    for path in relevant:
        row = {"path": path}
        normalized = []
        for rev in [BASE, CURRENT]:
            text = source(rev, path) if path in paths[rev] else ""
            prod = "" if test_only(path) else production(text) if path.endswith(".rs") else text
            norm = formatted(path, prod)
            row[rev] = {"physical": count(prod), "normalized": count(norm), "test": minus(count(text), count(prod))}
            normalized.append(norm.splitlines())
            group = "scope" if in_scope(path) else "other"
            for mode, value in [("physical", count(prod)), ("normalized", count(norm))]:
                totals[mode][rev][group] = plus(totals[mode][rev][group], value)
                totals[mode][rev]["tests"] = plus(totals[mode][rev]["tests"], row[rev]["test"])
                totals[mode][rev]["all"] = plus(totals[mode][rev]["all"], count(text))
        for tag, a, b, c, d in difflib.SequenceMatcher(None, *normalized, autojunk=False).get_opcodes():
            if tag == "equal": churn[2] += b-a
            else: churn[0] += b-a; churn[1] += d-c
        rows.append(row)
    # This historical reproduction guards against accidentally counting test
    # fixtures or losing the formerly excluded two-line attachment test import.
    # Per-item cfg(test) stripping retains five blank separators discarded by the
    # old test-module suffix shortcut; the 6,582 nonblank lines are unchanged.
    if BASE == "415aeff": assert totals["physical"][BASE]["scope"] == [6625, 6582], totals["physical"][BASE]
    print(json.dumps({"base": BASE, "current": CURRENT, "units": ["physical", "nonblank"], "totals": totals,
                      "normalized_per_file_churn": {"deleted": churn[0], "added": churn[1], "unchanged": churn[2]},
                      "files": rows}, indent=2))


if __name__ == "__main__": main()

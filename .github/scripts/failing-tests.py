"""Turns the failures in a `cargo test` log into GitHub annotations.

One annotation per failing test (its panic, message and `left`/`right`
lines), so they can be read on the run without the job log. GitHub keeps
10 error annotations per step: the first 9 tests, then one with the rest.
"""

import re
import sys

LINE_MAX = 300
LINES_MAX = 25


def escape(text: str) -> str:
    return text.replace("%", "%25").replace("\r", "").replace("\n", "%0A")


def escape_property(text: str) -> str:
    return escape(text).replace(":", "%3A").replace(",", "%2C")


def main(path: str) -> None:
    log = open(path, encoding="utf-8", errors="replace").read().splitlines()
    # libtest prints "---- <name> stdout ----" then the test's output and panic.
    sections: dict[str, list[str]] = {}
    name = None
    for line in log:
        m = re.match(r"^---- (\S+) stdout ----$", line)
        if m:
            name = m.group(1)
            sections[name] = []
        elif name and (line == "failures:" or line.startswith("test result:")):
            name = None
        elif name:
            sections[name].append(line)
    failed = [m.group(1) for line in log if (m := re.match(r"^test (\S+) \.\.\. FAILED", line))]
    for test in failed:
        sections.setdefault(test, [])
    if not sections:
        return
    names = list(sections)
    for test in names[:9]:
        body = sections[test]
        keep = body[:LINES_MAX] + [l for l in body[LINES_MAX:] if re.match(r"^\s*(left|right):", l)]
        text = "\n".join(l[:LINE_MAX] for l in keep) or "(no output)"
        print(f"::error title={escape_property(test)}::{escape(text)}")
    if len(names) > 9:
        print(f"::error title=More failing tests::{escape(chr(10).join(names[9:]))}")


if __name__ == "__main__":
    main(sys.argv[1])

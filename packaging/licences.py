#!/usr/bin/env python3
"""Writes the third-party licence notices shown in Help > About > Third-party licences.

Every crate built into Filescape on Windows, macOS or Linux is listed with its version, its
licence and the licence files it ships (or, for a crate that ships none, the standard text of its
licence with its authors). Identical texts are written once, under all the crates that use them.

    python3 packaging/licences.py           # rewrite packaging/third-party-licences.txt
    python3 packaging/licences.py --check   # fail if it's out of date (CI)

Run it after changing dependencies. Needs cargo and the network (cargo downloads every
platform's crates to read their licence files).
"""

import json
import os
import subprocess
import sys
import textwrap

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "packaging", "third-party-licences.txt")
TARGETS = ["x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc", "aarch64-apple-darwin", "x86_64-apple-darwin", "x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"]
PREFIXES = ("LICENSE", "LICENCE", "COPYING", "NOTICE", "COPYRIGHT", "UNLICENSE")

MIT = """Permission is hereby granted, free of charge, to any person obtaining a copy of this software and
associated documentation files (the "Software"), to deal in the Software without restriction,
including without limitation the rights to use, copy, modify, merge, publish, distribute,
sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or
substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT
NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES
OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE."""


# Crates that build C or C++ sources they carry, whose own notices sit in subfolders. Listed by
# hand because the script can't tell which of those sources end up linked in.
BUNDLED = {
    # llama.cpp itself (llama, ggml, common) and the vendored code common links in.
    "llama-cpp-sys-2": [
        "llama.cpp/LICENSE",
        "llama.cpp/vendor/cpp-httplib/LICENSE",
        "llama.cpp/vendor/hash/xxhash/LICENSE",
        "llama.cpp/vendor/hash/rotate-bits/LICENSE.md",
        "llama.cpp/vendor/hash/sha256/LICENSE",
    ],
}
# Header-only code bundled the same way that carries its notice in the header, not a file.
BUNDLED_TEXT = {
    "llama-cpp-sys-2": [
        "nlohmann/json (bundled in llama.cpp)\nCopyright (c) 2013-2025 Niels Lohmann <https://nlohmann.me>\n\n" + "MIT_PLACEHOLDER",
    ],
}


def metadata():
    cmd = ["cargo", "metadata", "--format-version", "1", "--manifest-path", os.path.join(ROOT, "Cargo.toml")]
    for t in TARGETS:
        cmd += ["--filter-platform", t]
    return json.loads(subprocess.run(cmd, check=True, capture_output=True, text=True).stdout)


def shipped(meta):
    """The packages built into the program: normal and build dependencies, not dev ones."""
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    seen, todo = set(), [meta["resolve"]["root"]]
    while todo:
        i = todo.pop()
        if i in seen:
            continue
        seen.add(i)
        for d in nodes[i]["deps"]:
            if any(k["kind"] in (None, "build") for k in d["dep_kinds"]):
                todo.append(d["pkg"])
    ours = set(meta["workspace_members"])
    pkgs = {p["id"]: p for p in meta["packages"]}
    # Path dependencies (our own crates) aren't third-party.
    return [pkgs[i] for i in seen if i not in ours and pkgs[i].get("source")]


def texts(p):
    """The licence texts a crate ships, or a stand-in for one that ships none."""
    d = os.path.dirname(p["manifest_path"])
    names = []
    if p.get("license_file"):
        names.append(p["license_file"])
    names += sorted(f for f in os.listdir(d) if f.upper().startswith(PREFIXES) and f not in names)
    # Bundled fonts carry their own licences (OFL, the Ubuntu font licence) beside them.
    if os.path.isdir(os.path.join(d, "fonts")):
        names += sorted("fonts/" + f for f in os.listdir(os.path.join(d, "fonts")) if f.lower().endswith(".txt"))
    names += [n for n in BUNDLED.get(p["name"], []) if n not in names]
    # A crate from a git workspace (craft-libs) keeps its licences at the workspace root.
    if not names and (p.get("source") or "").startswith("git+"):
        up = d
        while os.path.dirname(up) != up and not os.path.isdir(os.path.join(up, ".git")) and not os.path.isfile(os.path.join(up, ".cargo-ok")):
            up = os.path.dirname(up)
        names = sorted(os.path.relpath(os.path.join(up, f), d) for f in os.listdir(up) if f.upper().startswith(("LICENSE", "LICENCE")))
    out = []
    for n in names:
        f = os.path.join(d, n)
        if os.path.isfile(f):
            with open(f, encoding="utf-8", errors="replace") as fh:
                t = fh.read().replace("\r\n", "\n").strip("\n")
            if t.strip():
                out.append(t)
    if not out:
        authors = ", ".join(a.split("<")[0].strip() for a in p.get("authors") or []) or f"the {p['name']} authors"
        lic = p.get("license") or "see the crate"
        if "MIT" in lic:
            out.append(f"Copyright (c) {authors}\n\n{MIT}")
        else:
            out.append(f"Copyright (c) {authors}\n\nLicensed under {lic}. The crate ships no licence file; see {p.get('repository') or 'crates.io'}.")
    return out + [t.replace("MIT_PLACEHOLDER", MIT) for t in BUNDLED_TEXT.get(p["name"], [])]


def render(meta):
    pkgs = sorted(shipped(meta), key=lambda p: (p["name"].lower(), p["version"]))
    groups = {}  # text -> crates using it
    for p in pkgs:
        for t in texts(p):
            groups.setdefault(t, []).append(f"{p['name']} {p['version']}")
    lines = [
        "Filescape is built with the open-source software listed below. Each is used under the licence",
        "shown (where a crate offers a choice, Filescape uses it under any one of them). The full",
        "licence texts follow the list.",
        "",
    ]
    for p in pkgs:
        lines.append(f"{p['name']} {p['version']}\t{p.get('license') or 'see its licence text'}")
    lines.append("")
    # MPL-2.0 asks that people can find the source of what's distributed in executable form.
    mpl = [p for p in pkgs if "MPL" in (p.get("license") or "")]
    for p in mpl:
        lines += textwrap.wrap(
            f"{p['name']} {p['version']} is used unmodified. Its source code is at "
            f"https://crates.io/crates/{p['name']}/{p['version']}"
            + (f" and {p['repository']}." if p.get("repository") else "."),
            100,
            break_on_hyphens=False,
        )
    if mpl:
        lines.append("")
    for t, users in sorted(groups.items(), key=lambda g: g[1][0].lower()):
        lines.append("=" * 72)
        lines += textwrap.wrap("Used by: " + ", ".join(users), 100, break_on_hyphens=False)
        lines.append("=" * 72)
        lines.append(t)
        lines.append("")
    return "\n".join(lines)


def main():
    text = render(metadata())
    if "--check" in sys.argv:
        old = open(OUT, encoding="utf-8").read() if os.path.exists(OUT) else ""
        if old != text:
            sys.exit("packaging/third-party-licences.txt is out of date: run python3 packaging/licences.py")
        return
    with open(OUT, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    print(f"wrote {OUT} ({len(text) // 1024} KB)")


if __name__ == "__main__":
    main()

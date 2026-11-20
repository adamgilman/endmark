"""The Endmark compatibility matrix (RFC 0001, section 12.7).

Renders every clean conformance fixture through each Markdown engine twice:
the whole file, and the body alone. A record is hidden when the two give
the same HTML once HTML comments are removed.

Usage: python3 compat/run.py [--write] [--endmark PATH]
Needs: pip install -r compat/requirements.txt; npm install in compat/; cmark-gfm.
An engine that is not installed is reported as not run, and does not fail the check
unless --require-all is given.
Exits 1 if an engine that is expected to hide records shows one.
"""
import json
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent

# engine -> (command prefix, what is expected)
PY = [sys.executable, str(HERE / "render_py.py")]
JS = ["node", str(HERE / "render_js.mjs")]
ENGINES = {
    "commonmark-py": (PY + ["commonmark"], "hidden"),
    "markdown-it-py, HTML on": (PY + ["markdown-it-py"], "hidden"),
    "markdown-it-py, HTML off": (PY + ["markdown-it-py-html-off"], "visible"),
    "mistune": (PY + ["mistune"], "hidden"),
    "Python-Markdown": (PY + ["python-markdown"], "hidden"),
    "commonmark.js": (JS + ["commonmark.js"], "hidden"),
    "markdown-it (JavaScript), HTML on": (JS + ["markdown-it"], "hidden"),
    "markdown-it (JavaScript), HTML off, its default": (JS + ["markdown-it-html-off"], "visible"),
    "marked": (JS + ["marked"], "hidden"),
    "cmark-gfm, unsafe mode": (["cmark-gfm", "--unsafe"], "hidden"),
    "cmark-gfm, safe mode, its default": (["cmark-gfm"], "removed"),
}

# Files an engine is known to render differently, with the reason.
KNOWN = {
    ("Python-Markdown", "escapes-in-quotes"): (
        "the body contains an unclosed `<!--`. Python-Markdown is not a CommonMark engine: it reads that as"
        " the start of a comment, which swallows the start of the first record and leaves its end visible"
    ),
}

COMMENT = re.compile(r"<!--.*?-->", re.S)


def normalise(html: str) -> str:
    """Drops comments, then all whitespace, so only visible markup is compared."""
    html = COMMENT.sub("", html).replace("<!-- raw HTML omitted -->", "")
    return re.sub(r"\s+", "", html)


def fixtures():
    """Fixtures a reader accepts without errors, and that hold a record."""
    for md in sorted((ROOT / "conformance").glob("*.md")):
        expectation = md.with_suffix(".json")
        if md.name == "README.md" or not expectation.exists():
            continue
        found = json.loads(expectation.read_text(encoding="utf-8"))
        errors = [d for d in found["diagnostics"] if d["code"] in ERROR_CODES]
        if not errors and found["items"]:
            yield md


ERROR_CODES = {
    "early-close", "unterminated-record", "unterminated-fence", "trailing-text", "bad-header",
    "missing-attribute", "incomplete-edit", "missing-fence", "unexpected-fence", "duplicate-id",
    "unknown-target", "wrong-target", "not-author", "already-final", "visible-record",
}


def run(command, path):
    done = subprocess.run(command + [str(path)], capture_output=True, text=True, encoding="utf-8")
    if done.returncode != 0:
        raise RuntimeError(f"{' '.join(command)}: {done.stderr.strip()}")
    return done.stdout


def main():
    args = sys.argv[1:]
    endmark = args[args.index("--endmark") + 1] if "--endmark" in args else str(ROOT / "target/debug/endmark")
    files = list(fixtures())
    rows, failed = [], False
    with tempfile.TemporaryDirectory() as tmp:
        for name, (command, expected) in ENGINES.items():
            if shutil.which(command[0]) is None:
                rows.append((name, expected, "not run: " + command[0] + " is not installed", ""))
                failed |= "--require-all" in args
                continue
            shown = []
            for md in files:
                body = Path(tmp) / "body.md"
                body.write_bytes(subprocess.run([endmark, "body", str(md)], capture_output=True, check=True).stdout)
                if normalise(run(command, md)) != normalise(run(command, body)) and (name, md.stem) not in KNOWN:
                    shown.append(md.stem)
            if not shown:
                result = "hidden"
            elif expected == "hidden":
                result = "VISIBLE"
            else:
                result = "visible"
            if expected == "removed":
                result = "removed" if not shown else "VISIBLE"
            ok = result == expected
            failed |= not ok
            excused = [stem for (engine, stem) in KNOWN if engine == name]
            checked = len(files) - len(excused)
            note = f"{checked - len(shown)} of {checked} files identical to the body alone"
            if excused:
                note += ". Known exception: " + ", ".join(f"`{stem}`" for stem in excused)
            rows.append((name, expected, result if ok else f"**{result}**, expected {expected}", note + ("" if ok else ": " + ", ".join(shown[:5]))))

    table = ["| Engine | Expected | Result | Detail |", "| --- | --- | --- | --- |"]
    table += [f"| {a} | {b} | {c} | {d} |" for a, b, c, d in rows]
    text = "\n".join(table) + "\n"
    if KNOWN:
        text += "\n**Known exceptions**\n\n"
        text += "".join(f"- {engine}, `{stem}`: {why}.\n" for (engine, stem), why in KNOWN.items())
    print(text)
    if "--write" in args:
        (HERE / "RESULTS.md").write_text(HEADER + text, encoding="utf-8")
    sys.exit(1 if failed else 0)


HEADER = """# Compatibility results

Written by `python3 compat/run.py --write`. Do not edit by hand.

- **hidden**: the engine renders the file exactly as it renders the body alone.
- **visible**: the engine prints records as text. Expected where raw HTML is turned off (RFC 0001, D-4).
- **removed**: the engine drops raw HTML, records included, and the rest renders as the body alone.

"""

if __name__ == "__main__":
    main()

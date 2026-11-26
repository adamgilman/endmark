#!/usr/bin/env python3
"""Writes crates/endmark-cli/THIRD_PARTY_NOTICES.txt.

The endmark binary contains other people's code: the Rust crates it links,
and the JavaScript packages bundled into the review page it serves. Their
licenses ask that their notices travel with it. This script lists every one,
with its version, its license and the license files from its own package.
`endmark --licenses` prints the result, and the release archives carry it.

    python3 scripts/notices.py          # rewrite the file
    python3 scripts/notices.py --check  # fail if it is out of date

The Rust crates are the ones `cargo tree` says are linked into the binary or
the WebAssembly module, from the sources cargo has downloaded. The JavaScript
packages are the production dependencies in packages/render/bun.lock. Each
tarball is fetched from the npm registry and checked against the hash in the
lock file, so the script needs neither bun nor node_modules. Downloads are
kept in target/notices-cache.

It also checks that each published crate carries a copy of LICENSE.
"""

import base64
import hashlib
import io
import json
import re
import subprocess
import sys
import tarfile
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "crates/endmark-cli/THIRD_PARTY_NOTICES.txt"
LOCK = ROOT / "packages/render/bun.lock"
CACHE = ROOT / "target/notices-cache"
CRATES_WITH_LICENSE = ["crates/endmark-core", "crates/endmark-cli"]
OUR_CRATES = {"endmark", "endmark-core", "endmark-wasm"}
LICENSE_FILE = re.compile(r"^(licen[cs]e|copying|notice)([-._].*)?$", re.IGNORECASE)

# Said about a package beside its license, where the package itself does not
# make the terms plain.
NOTES = {
    "khroma": "khroma's package.json names no license. The license file in its package,\n"
    "below, is the MIT License.",
    "elkjs": "elkjs is offered under EPL-2.0 OR GPL-3.0-or-later. It is used here under the\n"
    "Eclipse Public License 2.0. Its source code is at\n"
    "https://github.com/kieler/elkjs, under the tag of the version above.",
}


def run(*args):
    return subprocess.run(args, cwd=ROOT, check=True, capture_output=True, text=True).stdout


def license_files(names_and_texts):
    found = [(name, text) for name, text in names_and_texts if LICENSE_FILE.match(name)]
    return sorted(found)


# Rust


def rust_packages():
    tree = run(
        "cargo", "tree", "--locked", "-p", "endmark", "-p", "endmark-wasm",
        "-e", "normal,no-proc-macro", "--target", "all", "--prefix", "none", "-f", "{p}",
    )
    wanted = set()
    for line in tree.splitlines():
        parts = line.split()
        if len(parts) >= 2 and parts[0] not in OUR_CRATES:
            wanted.add((parts[0], parts[1].lstrip("v")))
    metadata = json.loads(run("cargo", "metadata", "--locked", "--format-version", "1"))
    packages = []
    for package in metadata["packages"]:
        if (package["name"], package["version"]) not in wanted:
            continue
        directory = Path(package["manifest_path"]).parent
        files = license_files(
            (path.name, path.read_text(encoding="utf-8", errors="replace"))
            for path in directory.iterdir()
            if path.is_file()
        )
        packages.append({
            "name": package["name"],
            "version": package["version"],
            "license": package.get("license") or "",
            "source": package.get("repository") or f"https://crates.io/crates/{package['name']}",
            "files": files,
        })
        wanted.discard((package["name"], package["version"]))
    if wanted:
        sys.exit(f"notices: cargo metadata does not know {sorted(wanted)}")
    return sorted(packages, key=lambda p: (p["name"], p["version"]))


# JavaScript


def bun_lock():
    text = LOCK.read_text(encoding="utf-8")
    return json.loads(re.sub(r",(\s*[}\]])", r"\1", text))


def npm_packages():
    lock = bun_lock()
    entries = lock["packages"]
    stack = list(lock["workspaces"][""].get("dependencies", {}))
    seen = {}
    while stack:
        key = stack.pop()
        if key in seen or key not in entries:
            continue
        entry = entries[key]
        name, version = entry[0].rsplit("@", 1)
        meta = entry[2] if len(entry) > 2 and isinstance(entry[2], dict) else {}
        seen[key] = (name, version, entry[1], entry[3] if len(entry) > 3 else "")
        for dependency in [*meta.get("dependencies", {}), *meta.get("optionalDependencies", {})]:
            nested = f"{key}/{dependency}"
            stack.append(nested if nested in entries else dependency)
    unique = sorted(set(seen.values()))
    with ThreadPoolExecutor(16) as pool:
        return list(pool.map(npm_package, unique))


def tarball(name, version, registry, integrity):
    CACHE.mkdir(parents=True, exist_ok=True)
    cached = CACHE / f"{name.replace('/', '+')}-{version}.tgz"
    if cached.exists():
        data = cached.read_bytes()
    else:
        base = (registry or "https://registry.npmjs.org/").rstrip("/")
        url = f"{base}/{name}/-/{name.split('/')[-1]}-{version}.tgz"
        request = urllib.request.Request(url, headers={"User-Agent": "endmark-notices"})
        with urllib.request.urlopen(request, timeout=60) as response:
            data = response.read()
    algorithm, _, expected = integrity.partition("-")
    if algorithm != "sha512" or base64.b64encode(hashlib.sha512(data).digest()).decode() != expected:
        sys.exit(f"notices: {name}@{version} does not match the hash in bun.lock")
    cached.write_bytes(data)
    return data


def npm_package(spec):
    name, version, registry, integrity = spec
    data = tarball(name, version, registry, integrity)
    top = {}
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
        for member in archive.getmembers():
            parts = member.name.split("/")
            if member.isfile() and len(parts) == 2:
                top[parts[1]] = archive.extractfile(member).read().decode("utf-8", errors="replace")
    manifest = json.loads(top.get("package.json", "{}"))
    license = manifest.get("license") or ""
    if isinstance(license, dict):
        license = license.get("type", "")
    repository = manifest.get("repository") or ""
    if isinstance(repository, dict):
        repository = repository.get("url", "")
    return {
        "name": name,
        "version": version,
        "license": license,
        "source": repository or f"https://www.npmjs.com/package/{name}",
        "files": license_files(top.items()),
    }


# Output


def render(rust, npm):
    rule = "=" * 76
    out = [
        "Third-party software in endmark",
        "",
        "The endmark binary, and the review page built into it, contain the",
        "software listed below. Each is listed with its version and license,",
        "followed by the license files from its own package.",
        "",
        "Endmark itself is under the MIT License; see LICENSE.",
        "",
        "This file is generated by scripts/notices.py. Do not edit it by hand.",
        "",
        "Rust crates, linked into the binary and its WebAssembly module:",
        "",
    ]
    width = max(len(f"{p['name']} {p['version']}") for p in rust + npm) + 2
    def row(package):
        license = package["license"] or "see below"
        return f"  {package['name'] + ' ' + package['version']:<{width}}{license}"

    out += [row(p) for p in rust]
    out += ["", "JavaScript packages, bundled into the review page:", ""]
    out += [row(p) for p in npm]
    missing = []
    for package in rust + npm:
        out += ["", rule, f"{package['name']} {package['version']}", f"License: {package['license'] or 'see below'}"]
        out.append(f"Source: {package['source']}")
        if package["name"] in NOTES:
            out += ["", NOTES[package["name"]]]
        if not package["files"]:
            missing.append(f"{package['name']} {package['version']}")
        for file_name, text in package["files"]:
            out += ["", f"--- {file_name}", "", text.replace("\r\n", "\n").strip("\n")]
    if missing:
        sys.exit("notices: no license file in " + ", ".join(missing))
    return "\n".join(out) + "\n"


def main():
    check = sys.argv[1:] == ["--check"]
    if sys.argv[1:] not in ([], ["--check"]):
        sys.exit("usage: scripts/notices.py [--check]")
    problems = []
    license_text = (ROOT / "LICENSE").read_text(encoding="utf-8")
    for crate in CRATES_WITH_LICENSE:
        path = ROOT / crate / "LICENSE"
        if not path.exists() or path.read_text(encoding="utf-8") != license_text:
            if check:
                problems.append(f"{crate}/LICENSE is not a copy of LICENSE")
            else:
                path.write_text(license_text, encoding="utf-8")
    text = render(rust_packages(), npm_packages())
    if check:
        current = OUTPUT.read_text(encoding="utf-8") if OUTPUT.exists() else ""
        if current != text:
            problems.append(f"{OUTPUT.relative_to(ROOT)} is out of date")
        if problems:
            sys.exit("\n".join(problems) + "\nrun: python3 scripts/notices.py")
        print("notices: up to date")
    else:
        OUTPUT.write_text(text, encoding="utf-8")
        print(f"notices: wrote {OUTPUT.relative_to(ROOT)}")


if __name__ == "__main__":
    main()

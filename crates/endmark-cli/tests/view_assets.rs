//! The page of `endmark view` is built by `packages/render/build.ts` and
//! committed in `crates/endmark-cli/view/`, so that building the binary
//! needs neither bun nor the network. This test fails when what is
//! committed was built from other sources than the ones in the tree.
//!
//! To fix a failure: `cd packages/render && bun install && bun run build`,
//! and commit `crates/endmark-cli/view/`.
//!
//! It compares the sources, not the built files. Two versions of rustc or
//! of bun build different bytes from the same sources, so comparing a
//! fresh build with the committed one would fail for no reason.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The same list as `INPUTS` in `packages/render/build.ts`.
const DIRECTORIES: &[&str] = &[
    "crates/endmark-core/src",
    "crates/endmark-wasm/src",
    "packages/render/src",
    "view",
];
const FILES: &[&str] = &[
    "packages/render/build.ts",
    "packages/render/bun.lock",
    "packages/render/package.json",
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn files_under(directory: &Path, relative: &str, found: &mut Vec<String>) {
    let mut entries: Vec<_> = std::fs::read_dir(directory)
        .unwrap_or_else(|e| panic!("{}: {e}", directory.display()))
        .map(|entry| entry.expect("a directory entry"))
        .collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let name = format!("{relative}/{}", entry.file_name().to_string_lossy());
        if entry.file_type().expect("a file type").is_dir() {
            files_under(&entry.path(), &name, found);
        } else {
            found.push(name);
        }
    }
}

/// One SHA-256 over every input: for each file in order of its path, the
/// path, a zero byte, the bytes with CRLF read as LF, and a zero byte.
fn inputs_digest() -> String {
    let root = root();
    let mut paths: Vec<String> = FILES.iter().map(|f| f.to_string()).collect();
    for directory in DIRECTORIES {
        files_under(&root.join(directory), directory, &mut paths);
    }
    paths.sort();
    let mut all = Vec::new();
    for path in &paths {
        let text = std::fs::read_to_string(root.join(path)).unwrap_or_else(|e| panic!("{path}: {e}"));
        all.extend_from_slice(path.as_bytes());
        all.push(0);
        all.extend_from_slice(text.replace("\r\n", "\n").as_bytes());
        all.push(0);
    }
    sha256(&all).iter().map(|byte| format!("{byte:02x}")).collect()
}

/// SHA-256 (FIPS 180-4), written out here so that the test needs no crate.
fn sha256(message: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
        0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
        0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
        0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
        0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
        0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let mut padded = message.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&((message.len() as u64) * 8).to_be_bytes());
    for block in padded.chunks(64) {
        let mut w = [0u32; 64];
        for (i, word) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(choice)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(majority);
            (h, g, f, e, d, c, b, a) = (g, f, e, d.wrapping_add(t1), c, b, a, t1.wrapping_add(t2));
        }
        for (value, add) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *value = value.wrapping_add(add);
        }
    }
    let mut out = [0u8; 32];
    for (chunk, value) in out.chunks_mut(4).zip(state) {
        chunk.copy_from_slice(&value.to_be_bytes());
    }
    out
}

#[test]
fn sha256_is_sha256() {
    let hex = |bytes: &[u8]| -> String { sha256(bytes).iter().map(|b| format!("{b:02x}")).collect() };
    assert_eq!(
        hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        hex(&[b'a'; 1_000_000]),
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
}

#[test]
fn the_committed_page_was_built_from_the_sources_in_the_tree() {
    let view = root().join("crates/endmark-cli/view");
    let listing = std::fs::read_to_string(view.join("assets.rs")).expect("crates/endmark-cli/view/assets.rs");
    let recorded = listing
        .lines()
        .find_map(|line| line.strip_prefix("// inputs: sha256:"))
        .expect("assets.rs records what it was built from");
    assert_eq!(
        recorded,
        inputs_digest(),
        "\n\ncrates/endmark-cli/view/ is stale: the renderer, the page, the core or the wasm crate changed \
         after it was built.\nRun `bun install && bun run build` in packages/render and commit crates/endmark-cli/view/.\n"
    );
}

#[test]
fn the_directory_holds_what_the_listing_names_and_nothing_else() {
    let view = root().join("crates/endmark-cli/view");
    let listing = std::fs::read_to_string(view.join("assets.rs")).unwrap();
    let mut named: BTreeSet<String> = ["assets.rs", "index.html"].map(String::from).into();
    let mut paths = BTreeSet::new();
    for line in listing.lines().filter(|line| line.trim_start().starts_with("(\"/")) {
        let parts: Vec<&str> = line.split('"').collect();
        let (path, content_type, file) = (parts[1], parts[3], parts[5]);
        assert_eq!(format!("{}.gz", &path[1..]), file, "{line}");
        let expected = match path.rsplit('.').next() {
            Some("js") => "text/javascript; charset=utf-8",
            Some("css") => "text/css; charset=utf-8",
            Some("wasm") => "application/wasm",
            other => panic!("{path}: a file of a kind the server has no type for: {other:?}"),
        };
        assert_eq!(content_type, expected, "{line}");
        let bytes = std::fs::read(view.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
        // gzip, with no time and no file name in its header.
        assert_eq!(&bytes[..4], &[0x1f, 0x8b, 8, 0], "{file}");
        assert_eq!(&bytes[4..8], &[0, 0, 0, 0], "{file} has a time in its header");
        assert!(paths.insert(path.to_string()), "{path} is listed twice");
        named.insert(file.to_string());
    }
    assert!(paths.contains("/endmark.wasm"));
    assert!(paths.len() >= 3, "a script, a stylesheet and the wasm module at least");

    let present: BTreeSet<String> = std::fs::read_dir(&view)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        present, named,
        "files in crates/endmark-cli/view/ and files assets.rs names"
    );

    // The page names its script and stylesheet, which are among the files, and holds the place for the token.
    let page = std::fs::read_to_string(view.join("index.html")).unwrap();
    assert_eq!(page.matches("__ENDMARK_TOKEN__").count(), 1);
    for attribute in ["src=\"./", "href=\"./"] {
        let name = page
            .split(attribute)
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .unwrap();
        assert!(paths.contains(&format!("/{name}")), "the page names {name}");
    }
    assert!(
        !page.contains("http://") && !page.contains("https://") && !page.contains("src=\"//"),
        "the page names nothing outside itself"
    );
    // The whole page fits comfortably in a crate, which may be 10 MB.
    let total: u64 = present
        .iter()
        .map(|name| std::fs::metadata(view.join(name)).unwrap().len())
        .sum();
    assert!(total < 4 * 1024 * 1024, "crates/endmark-cli/view/ is {total} bytes");
}

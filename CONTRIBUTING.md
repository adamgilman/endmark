# Contributing

Endmark is a file format and a reference implementation. Contributions to either are welcome, under the MIT licence.

## Changing the format

The format is defined in [RFC 0001](rfcs/0001-endmark.md). To change a rule or add one:

1. Open an issue that says what goes wrong today, with a small example file.
2. Change the RFC, the implementation and the fixtures in one pull request, so the three never disagree.

Sections 13 and 14 of the RFC list the decisions that are still open and the rules added since the first draft.

## Changing the code

You need a Rust toolchain, 1.80 or later.

```sh
cargo test                                  # unit tests, fixtures, property test
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
```

CI runs the same three commands, on Linux, macOS and Windows.

- `crates/endmark-core/` is the library: no I/O, no dependencies beyond `pulldown-cmark`.
- `crates/endmark-cli/` is the `endmark` command.
- `crates/endmark-wasm/` is the library as a WebAssembly module. The renderer in `packages/render/` and the demo page in `demo/` are TypeScript on top of it, built and tested with [Bun](https://bun.sh); their READMEs give the commands.
- `view/` is the page that `endmark view` serves. Its built form is committed in `crates/endmark-cli/view/` and embedded in the binary. After changing the renderer, `view/`, or the source of `endmark-core` or `endmark-wasm`, run `bun run build` in `packages/render/` and commit that directory. `cargo test` fails until you do.
- `eval/` measures repair suggestions against the histories of real documents. Run `eval/run.sh` before and after changing `crates/endmark-core/src/repair.rs`, and add a scenario to `crates/endmark-core/tests/repair_scenarios.rs` for any kind of edit it gets wrong.
- `crates/endmark-cli/THIRD_PARTY_NOTICES.txt` lists the license of every crate and JavaScript package built into the binary, and `endmark --licenses` prints it. After changing `Cargo.lock` or `packages/render/bun.lock`, run `python3 scripts/notices.py` and commit what it writes. CI fails until you do.
- `spec/endmark.md` is generated from the RFC. After changing the RFC, run `UPDATE_SPEC=1 cargo test --test spec`.

## Fixtures

Every rule has a fixture in `conformance/`, and a change in behaviour needs one. [conformance/README.md](conformance/README.md) gives the format.

Write the expected output by hand, from the RFC. Do not paste what the implementation prints: a fixture that was captured from the code can only show that the code has not changed.

## Conduct

Everyone taking part follows the [code of conduct](CODE_OF_CONDUCT.md).

## Reporting a bug

A bug report is most useful as a file and a command: the smallest Markdown file that shows the problem, the `endmark` command you ran, what it printed, and what you expected.

For a security problem, see [SECURITY.md](SECURITY.md).

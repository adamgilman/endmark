# Endmark reference renderer

This package draws an Endmark file in a web page. It implements section 11 of `rfcs/0001-endmark.md`.

It holds no logic for the format. Every read goes through `endmark-core`, compiled to WebAssembly from `crates/endmark-wasm`. The renderer writes nothing. Each action a reader takes is passed to a callback.

## What is here

| Path | Purpose |
| --- | --- |
| `src/core.ts` | Loads the wasm module and exposes `call(request)`. |
| `src/render.ts` | `mount(container, source, options)`. Draws the four views: Final, Review, Preview and History. |
| `src/blocks.ts` | Markdown handling with no DOM: splits the body into blocks and carries highlights through `markdown-it`. |
| `src/model.ts` | Arranges the items the core reports: threads, units of blocks, history order. |
| `src/bytes.ts` | Converts the core's byte ranges to string indices. |
| `src/diff.ts` | Word and line diffs. |
| `src/mermaid.ts` | Loads `mermaid` on first use. |
| `src/style.css` | The styles. Light and dark. |
| `build.ts` | Builds the wasm module, the demo page, and the page of `endmark view`. |
| `serve.ts` | Serves the built demo on localhost. |
| `test/` | Unit tests, a smoke test that runs the built demo in headless Chromium, and one that runs `endmark view`. |

Two pages are built on the renderer. `demo/` holds a document in memory beside its source. `view/` is the page that `endmark view FILE` serves: it reads the file from that server and sends each action to it.

## Use

```ts
import { loadCore, mount } from "./src/index";

const core = await loadCore(new URL("endmark.wasm", document.baseURI));
const renderer = mount(document.querySelector("#doc")!, source, {
  core,
  me: "you",
  actions: {
    accept: (id) => { /* ask the core to write, then call renderer.update(newSource) */ },
  },
});
```

An action that has no callback is not offered. Include `src/style.css` in the page.

An action may return nothing, an outcome `{ ok: boolean }`, or a promise of either. A host that writes somewhere else, such as a server, returns a promise. A form stays open, with what was typed in it, when the outcome has `ok: false`.

`renderer.update(source)` may be called at any time, for example when the file changed on disk. A form that is open is carried over with its text and its caret.

## Build

You need Rust with the `wasm32-unknown-unknown` target, and Bun 1.4 or later.

```sh
cd packages/render
bun install
bun run build
```

`bun run build` runs `cargo build --release -p endmark-wasm --target wasm32-unknown-unknown` and then bundles two pages:

- the demo page into `demo/dist/`, which is not committed;
- the page of `endmark view` into `crates/endmark-cli/view/`, which **is** committed. The `endmark` binary embeds those files, so `cargo install endmark` needs neither Bun nor this build. Every file but `index.html` is stored compressed with gzip, and `assets.rs` lists them.

After you change the renderer, `view/`, `crates/endmark-core/src/` or `crates/endmark-wasm/src/`, run `bun run build` and commit `crates/endmark-cli/view/`. `cargo test` fails until you do: `crates/endmark-cli/tests/view_assets.rs` compares a hash of those sources with the one recorded in `assets.rs`. It compares sources and not built files, because two versions of rustc or Bun build different bytes from the same sources. With one toolchain the build is reproducible: building twice gives identical files.

## Run locally

```sh
bun run serve
```

This serves `demo/dist/` at `http://127.0.0.1:4173/`. Set `PORT` to use another port. Any static file server works as well.

## Test

```sh
bun test          # unit tests; needs the wasm module, so build first
bun run smoke     # the built demo in headless Chromium; needs `bun run build`
bun run smoke:view  # `endmark view` in headless Chromium; builds the binary with cargo
bun run typecheck
```

`bun test` loads the real wasm module. It checks `status` and `accept` against the fixtures in `conformance/`, the pure functions, and the demo's actions.

`bun run smoke` starts Chromium through the DevTools protocol. It clicks the page's buttons and selects text as a visitor would. It fails on any console error and on any request made after the page has loaded. Set `CHROMIUM` if the browser is not on the path as `chromium`.

`bun run smoke:view` starts the real `endmark view` on a copy of the worked example and reads the file on disk after each action. It also changes the file from outside while a reply is half typed, opens a read-only server, and opens a document full of hostile HTML. It fails on any console error, on any request to another origin, and if a script from the document runs.

## Limits

- Raw HTML in the body is shown as text, the same as in notes.
- A highlight or an in-place edit that would break the Markdown around it is not drawn in place. A comment then shows its quote in its card. An edit is shown as the old block with the suggested block under it.
- Added elements are marked in flowcharts only, by comparing node labels.
- Text selected inside a diagram, a stacked alternative or a line diff cannot be commented on.

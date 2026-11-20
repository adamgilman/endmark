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
| `build.ts` | Builds the wasm module and the demo page. |
| `serve.ts` | Serves the built demo on localhost. |
| `test/` | Unit tests, and a smoke test that runs the built demo in headless Chromium. |

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

## Build

You need Rust with the `wasm32-unknown-unknown` target, and Bun 1.4 or later.

```sh
cd packages/render
bun install
bun run build
```

`bun run build` runs `cargo build --release -p endmark-wasm --target wasm32-unknown-unknown` and then bundles the demo page into `demo/dist/`. The output is not committed.

## Run locally

```sh
bun run serve
```

This serves `demo/dist/` at `http://127.0.0.1:4173/`. Set `PORT` to use another port. Any static file server works as well.

## Test

```sh
bun test          # unit tests; needs the wasm module, so build first
bun run smoke     # the built demo in headless Chromium; needs `bun run build`
bun run typecheck
```

`bun test` loads the real wasm module. It checks `status` and `accept` against the fixtures in `conformance/`, the pure functions, and the demo's actions.

`bun run smoke` starts Chromium through the DevTools protocol. It clicks the page's buttons and selects text as a visitor would. It fails on any console error and on any request made after the page has loaded. Set `CHROMIUM` if the browser is not on the path as `chromium`.

## Limits

- Raw HTML in the body is shown as text, the same as in notes.
- A highlight or an in-place edit that would break the Markdown around it is not drawn in place. A comment then shows its quote in its card. An edit is shown as the old block with the suggested block under it.
- Added elements are marked in flowcharts only, by comparing node labels.
- Text selected inside a diagram, a stacked alternative or a line diff cannot be commented on.

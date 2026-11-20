# Endmark demo page

One static page that shows the Relay example from Appendix A of `rfcs/0001-endmark.md`. The document is on the left, drawn by the reference renderer in Review view. The raw source is on the right. On a narrow screen the two are stacked. It implements section 12.5 of the RFC.

The page holds no logic for the format. Every action calls `endmark-core`, compiled to WebAssembly. It saves nothing and makes no network request after it has loaded. A reload returns the original example.

## What is here

| Path | Purpose |
| --- | --- |
| `index.html` | The page. |
| `src/main.ts` | Wires the renderer and the session to the page. |
| `src/session.ts` | The page's state and its actions. No DOM, so the tests call it directly. |
| `src/demo.css` | Page layout. It imports the renderer's styles. |
| `dist/` | Build output. Not committed. |

## Build

```sh
cd packages/render
bun install
bun run build
```

This builds the wasm module and writes a self-contained `demo/dist/`. Copy that directory to any static host.

## Run locally

```sh
cd packages/render
bun run serve
```

Open `http://127.0.0.1:4173/`. The page must be served over HTTP. Opening `index.html` from disk does not work, because browsers do not load modules or wasm from `file:` URLs.

## Test

```sh
cd packages/render
bun test
bun run smoke
```

`bun test` drives `src/session.ts`: accept, revert, restore, comment, suggest, and a direct body edit. `bun run smoke` opens the built page in headless Chromium and does the same through the page's buttons.

## Notes

- Actions are attributed to `by=you`, with the time from the browser's clock.
- A new id is a type letter, a hyphen and four random characters. If the id is in use, the page draws another.
- "Edit the body directly" replaces the body text and writes no record. It works while the file starts with its body, which is always true for this example.
- Mermaid is split into chunks in `dist/`. The page loads the flowchart chunks when it first draws the example. If you edit the body into another diagram type, the browser fetches that type's chunk from the same directory.

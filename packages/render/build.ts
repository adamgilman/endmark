// Builds the wasm module and the demo page into demo/dist.
// Run with `bun run build` in packages/render.

import { $ } from "bun";
import { cp, rm } from "node:fs/promises";
import { join, resolve } from "node:path";

const root = resolve(import.meta.dir, "../..");
const dist = join(root, "demo/dist");
const wasm = join(root, "target/wasm32-unknown-unknown/release/endmark_wasm.wasm");

await $`cargo build --release -p endmark-wasm --target wasm32-unknown-unknown`.cwd(root);

await rm(dist, { recursive: true, force: true });
const result = await Bun.build({
  entrypoints: [join(root, "demo/index.html")],
  outdir: dist,
  target: "browser",
  splitting: true,
  minify: true,
});
if (!result.success) {
  for (const log of result.logs) console.error(log);
  process.exit(1);
}
await cp(wasm, join(dist, "endmark.wasm"));

const files = [...new Bun.Glob("**/*").scanSync(dist)];
console.log(`Built ${files.length} files into ${dist}`);

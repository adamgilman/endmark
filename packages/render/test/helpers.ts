import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { loadCore, type Core } from "../src/core";

export const root = resolve(import.meta.dir, "../../..");

export function fixture(name: string): string {
  return readFileSync(join(root, "conformance", name), "utf8");
}

/** The real module. Build it first: `bun run build`, or the cargo command in the README. */
export function core(): Promise<Core> {
  const path = join(root, "target/wasm32-unknown-unknown/release/endmark_wasm.wasm");
  return loadCore(readFileSync(path));
}

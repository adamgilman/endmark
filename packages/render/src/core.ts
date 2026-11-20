// Loads endmark-core, compiled to WebAssembly, and exposes its one call.
// The module has no imports. A request is JSON in, JSON out.
// See crates/endmark-wasm/src/lib.rs for the list of requests.

import type { PreviewOk, Refusal, Request, StatusOk, WriteOk } from "./types";

export type Response = StatusOk | PreviewOk | WriteOk | Refusal;

export interface Core {
  /** Answers one request. Never throws for a refused write: check `ok`. */
  call<T extends { ok: true } = StatusOk | PreviewOk | WriteOk>(request: Request): T | Refusal;
}

interface Exports {
  memory: WebAssembly.Memory;
  endmark_alloc(len: number): number;
  endmark_call(ptr: number, len: number): number;
  endmark_free(ptr: number, len: number): void;
}

export type WasmSource = string | URL | ArrayBuffer | ArrayBufferView | WebAssembly.Module;

/**
 * Loads the module. `wasm` is the bytes, a compiled module, or a URL to fetch.
 */
export async function loadCore(wasm: WasmSource): Promise<Core> {
  let module: WebAssembly.Module;
  if (wasm instanceof WebAssembly.Module) {
    module = wasm;
  } else if (typeof wasm === "string" || wasm instanceof URL) {
    const response = await fetch(wasm);
    if (!response.ok) throw new Error(`could not fetch ${wasm}: ${response.status}`);
    module = await WebAssembly.compile(await response.arrayBuffer());
  } else {
    module = await WebAssembly.compile(wasm as BufferSource);
  }
  const instance = await WebAssembly.instantiate(module, {});
  return coreFrom(instance.exports as unknown as Exports);
}

function coreFrom(wasm: Exports): Core {
  const encoder = new TextEncoder();
  const decoder = new TextDecoder();
  return {
    call<T>(request: Request): T | Refusal {
      const bytes = encoder.encode(JSON.stringify(request));
      const ptr = wasm.endmark_alloc(bytes.length);
      // The buffer can move when memory grows, so take a fresh view each time.
      new Uint8Array(wasm.memory.buffer, ptr, bytes.length).set(bytes);
      const out = wasm.endmark_call(ptr, bytes.length);
      const length = new DataView(wasm.memory.buffer).getUint32(out, true);
      const text = decoder.decode(new Uint8Array(wasm.memory.buffer, out + 4, length));
      wasm.endmark_free(out, 4 + length);
      return JSON.parse(text) as T | Refusal;
    },
  };
}

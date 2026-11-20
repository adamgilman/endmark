// Mermaid is loaded on first use, so a document without diagrams never pays for it.

export interface MermaidApi {
  initialize(config: Record<string, unknown>): void;
  parse(text: string): Promise<unknown>;
  render(id: string, text: string): Promise<{ svg: string }>;
}

let loaded: Promise<MermaidApi> | undefined;

export function loadMermaid(): Promise<MermaidApi> {
  loaded ??= import("mermaid").then((module) => {
    const api = module.default as unknown as MermaidApi;
    const dark = typeof matchMedia === "function" && matchMedia("(prefers-color-scheme: dark)").matches;
    api.initialize({ startOnLoad: false, securityLevel: "strict", theme: dark ? "dark" : "default" });
    return api;
  });
  return loaded;
}

// Mermaid is loaded on first use, so a document without diagrams never pays for it.

export interface MermaidApi {
  initialize(config: Record<string, unknown>): void;
  parse(text: string): Promise<unknown>;
  render(id: string, text: string): Promise<{ svg: string }>;
}

let loaded: Promise<MermaidApi> | undefined;

export function loadMermaid(): Promise<MermaidApi> {
  loaded ??= import("mermaid").then((module) => module.default as unknown as MermaidApi);
  return loaded;
}

/**
 * The colour scheme the page is drawn in: `data-theme` on the root element
 * when the host sets it to "light" or "dark", and otherwise the reader's
 * system setting. The stylesheet follows the same rule.
 */
export function pageTheme(): "light" | "dark" {
  const chosen = typeof document === "undefined" ? undefined : document.documentElement.dataset.theme;
  if (chosen === "light" || chosen === "dark") return chosen;
  return typeof matchMedia === "function" && matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

/** Sets Mermaid up for the page as it is now. Called before every round of drawing, so diagrams follow a change of theme. */
export function configureMermaid(api: MermaidApi): void {
  api.initialize({ startOnLoad: false, securityLevel: "strict", theme: pageTheme() === "dark" ? "dark" : "default" });
}

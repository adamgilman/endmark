// The theme switch in the header: Auto follows the system setting, Light
// and Dark override it. The choice is kept in this browser's storage for
// the page's origin, so it applies the next time `endmark view` runs on
// the same port. It is a convenience: if storage is unavailable the switch
// still works for this visit.

export type Choice = "auto" | "light" | "dark";

const KEY = "endmark-view-theme";

function stored(): Choice {
  try {
    const value = localStorage.getItem(KEY);
    return value === "light" || value === "dark" ? value : "auto";
  } catch {
    return "auto";
  }
}

function store(choice: Choice): void {
  try {
    if (choice === "auto") localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, choice);
  } catch {
    // Not kept for next time. Nothing else depends on it.
  }
}

/** Sets the page's theme. Auto removes the attribute, so the system setting decides. */
function apply(choice: Choice): void {
  if (choice === "auto") delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = choice;
}

// Applied as soon as this module runs, before the document is drawn.
apply(stored());

/**
 * Wires up the three buttons in `group`. `changed` is called whenever the
 * colours the page is drawn in may have changed: a different choice, or the
 * system setting changing while Auto is chosen.
 */
export function themeSwitch(group: HTMLElement, changed: () => void): void {
  const buttons = [...group.querySelectorAll<HTMLButtonElement>("button[data-theme]")];
  let current = stored();
  const show = (): void => {
    for (const button of buttons) button.setAttribute("aria-pressed", String(button.dataset.theme === current));
  };
  show();
  for (const button of buttons) {
    button.addEventListener("click", () => {
      const choice = button.dataset.theme as Choice;
      if (choice === current) return;
      current = choice;
      apply(choice);
      store(choice);
      show();
      changed();
    });
  }
  if (typeof matchMedia === "function") {
    matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
      if (current === "auto") changed();
    });
  }
}

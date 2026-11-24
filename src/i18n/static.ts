/**
 * Binds markup placeholders (`data-i18n*`) to the active dictionary.
 *
 * Static text lives only here and in the dictionaries; `index.html` keeps
 * structure and empty text nodes.
 */
import { hasMessage, t } from "./index";

const BINDINGS = [
  ["data-i18n", "textContent"],
  ["data-i18n-title", "title"],
  ["data-i18n-aria-label", "aria-label"],
  ["data-i18n-placeholder", "placeholder"],
] as const;

export function applyStaticTranslations(root: ParentNode = document): void {
  for (const [attribute, property] of BINDINGS) {
    for (const element of root.querySelectorAll<HTMLElement>(`[${attribute}]`)) {
      const key = element.getAttribute(attribute);
      if (key === null || !hasMessage(key)) continue;
      const text = t(key);
      if (property === "textContent") element.textContent = text;
      else element.setAttribute(property, text);
    }
  }
}

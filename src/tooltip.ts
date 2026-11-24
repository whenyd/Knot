/**
 * The native title tooltip's border and radius are drawn by the system, so CSS
 * cannot change them. When the pointer enters an element with a title, take the
 * text over into a custom overlay and put it back on leave or scroll.
 */
const SHOW_DELAY_MS = 400;

export function initTooltips(): void {
  const tip = document.createElement("div");
  tip.className = "hover-tip";
  tip.hidden = true;
  tip.setAttribute("role", "tooltip");
  document.body.append(tip);

  let host: HTMLElement | null = null;
  let text = "";
  let timer: number | undefined;
  let pointerX = 0;
  let pointerY = 0;

  const restoreTitle = (): void => {
    if (!host || host.dataset.tooltip === undefined) return;
    host.title = host.dataset.tooltip;
    delete host.dataset.tooltip;
  };

  const hide = (release: boolean): void => {
    window.clearTimeout(timer);
    timer = undefined;
    tip.hidden = true;
    if (release) {
      restoreTitle();
      host = null;
    }
  };

  const place = (): void => {
    const margin = 8;
    const left = Math.min(window.innerWidth - tip.offsetWidth - margin, pointerX + 12);
    const top = Math.min(window.innerHeight - tip.offsetHeight - margin, pointerY + 18);
    tip.style.left = `${Math.max(margin, left)}px`;
    tip.style.top = `${Math.max(margin, top)}px`;
  };

  document.addEventListener("pointerover", (event) => {
    if (event.pointerType === "touch") return;
    const target = event.target;
    if (!(target instanceof Element)) return;

    const next = target.closest<HTMLElement>("[title], [data-tooltip]");
    if (next === host) return;

    hide(true);
    if (!next) return;

    const nextText = next.getAttribute("title") ?? next.dataset.tooltip ?? "";
    if (nextText.trim() === "") return;

    host = next;
    text = nextText;
    if (next.hasAttribute("title")) {
      next.dataset.tooltip = nextText;
      next.removeAttribute("title");
    }
    pointerX = event.clientX;
    pointerY = event.clientY;
    timer = window.setTimeout(() => {
      if (!host) return;
      tip.textContent = text;
      tip.hidden = false;
      place();
    }, SHOW_DELAY_MS);
  });

  document.addEventListener("pointermove", (event) => {
    if (!host) return;
    pointerX = event.clientX;
    pointerY = event.clientY;
    if (!tip.hidden) place();
  });

  document.addEventListener("pointerout", (event) => {
    if (!host) return;
    const related = event.relatedTarget;
    if (related instanceof Node && host.contains(related)) return;
    hide(true);
  });

  document.addEventListener("pointerdown", () => hide(true));
  document.addEventListener("wheel", () => hide(true), { passive: true });
  window.addEventListener("blur", () => hide(true));
  window.addEventListener("resize", () => hide(true));
  window.addEventListener("scroll", () => hide(true), true);
}

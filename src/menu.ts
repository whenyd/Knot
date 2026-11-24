/**
 * Shared popup menu: the sidebar context menu, task-row context menu and
 * priority popover all use this one implementation.
 *
 * It only does three things — open, position, close; callers supply item
 * content and behavior. Positioning supports two modes: a pointer point (for
 * right-clicks) and an element anchor (below a button).
 *
 * Items with children no longer expand inline; instead a separate side panel
 * opens next to the parent row, so the parent's layout never shifts and closing
 * the submenu cannot yank the target out from under the pointer.
 */

export interface MenuItem {
  label: string;
  /** Action of a leaf item. Clicking an item with `children` only opens/closes its side panel; it never runs this. */
  onSelect?: () => void;
  /**
   * Children: the item becomes an expandable group; hover, click or keyboard
   * right-arrow opens the side panel. Nestable, for long lists like
   * "project → module → submodule".
   */
  children?: readonly MenuItem[];
  /** Current selection: shows a check and makes screen readers announce menuitemradio. */
  checked?: boolean;
  /** Decoration that is not a menu row: separator / group header. Defaults to a normal menu item. */
  type?: "item" | "separator" | "header";
}

export type MenuPosition =
  | { kind: "point"; x: number; y: number }
  | { kind: "anchor"; element: HTMLElement };

export interface MenuOptions {
  items: readonly MenuItem[];
  position: MenuPosition;
  /** Accessible name of the menu, e.g. "Priority". Sub-panels append to it. */
  label?: string;
  /** Who gets focus back on close; defaults to the anchor when anchored to an element. */
  restoreFocus?: HTMLElement;
  /** Extra root-panel class for special looks like the project menu. */
  className?: string;
  /** Called once when this menu closes for any reason. */
  onClose?: () => void;
}

interface PanelEntry {
  item: MenuItem;
  button: HTMLButtonElement;
  /** Side panel opened from this row; null when closed. */
  child: Panel | null;
}

interface Panel {
  element: HTMLElement;
  level: number;
  name: string;
  parent: Panel | null;
  parentEntry: PanelEntry | null;
  parentRow: HTMLButtonElement | null;
  entries: PanelEntry[];
}

interface OpenMenu {
  /** Always one path: panels[i].parent === panels[i - 1]. */
  panels: Panel[];
  anchor: HTMLElement | null;
  restoreFocus: HTMLElement | null;

  /** Intent timer for opening a side panel on hover. */
  intentTimer: number | null;
  intentEntry: PanelEntry | null;
  /** Timer for closing a row's side panel after the pointer leaves it. */
  closeTimer: number | null;
  /** Timer for the pointer leaving the entire menu tree. */
  leaveTimer: number | null;

  cleanup: () => void;
}

let open: OpenMenu | null = null;

// Programmatic focus after a mouse click can still match `:focus-visible` and
// leave the first item looking hovered. Track the input modality so only real
// keyboard navigation paints the focus background.
function markKeyboard(): void {
  document.body.classList.add("is-keyboard-nav");
}
function markPointer(): void {
  document.body.classList.remove("is-keyboard-nav");
}
window.addEventListener("keydown", markKeyboard, { capture: true });
window.addEventListener("pointerdown", markPointer, { capture: true });
window.addEventListener("pointermove", markPointer, { capture: true, passive: true });

const GAP = 2;
const EDGE = 4;
const MENU_PADDING = 4;
const BASE_Z_INDEX = 30;
const HOVER_OPEN_DELAY = 100;
const HOVER_CLOSE_DELAY = 150;
const LEAVE_DELAY = 200;

function isHovered(panel: Panel): boolean {
  return panel.element.matches(":hover");
}

function anyPanelHovered(): boolean {
  return open?.panels.some(isHovered) ?? false;
}

function menuContains(node: Node): boolean {
  return open?.panels.some((panel) => panel.element.contains(node)) ?? false;
}

/** Which panel the event target is in; keyboard navigation picks items from the current panel. */
function panelFor(node: Node): Panel | null {
  if (!open) return null;
  for (let index = open.panels.length - 1; index >= 0; index -= 1) {
    if (open.panels[index].element.contains(node)) return open.panels[index];
  }
  return null;
}

/**
 * Open the menu. Empty `items` does nothing — "nothing to do here" should not
 * pop an empty box.
 */
export function openMenu(options: MenuOptions): void {
  const { items, position } = options;
  const anchor = position.kind === "anchor" ? position.element : null;

  // Clicking the same anchor again = collapse (button semantics).
  if (anchor !== null && open?.anchor === anchor) {
    closeMenu();
    return;
  }

  closeMenu();
  if (items.length === 0) return;

  const session: OpenMenu = {
    panels: [],
    anchor,
    restoreFocus: options.restoreFocus ?? anchor,
    intentTimer: null,
    intentEntry: null,
    closeTimer: null,
    leaveTimer: null,
    cleanup: () => {},
  };
  open = session;

  const root = createPanel(session, items, 0, options.label ?? "", null, null, options.className);
  document.body.append(root.element);
  session.panels.push(root);
  placeRoot(root.element, position);

  // Panel events live on their own panels; only one global copy is kept.
  document.addEventListener("mousedown", onOutside);
  document.addEventListener("keydown", onEscape);
  document.addEventListener("keydown", onMenuKeydown);
  window.addEventListener("scroll", onWindowScroll, { capture: true });
  window.addEventListener("resize", closeMenu, { once: true });
  window.addEventListener("blur", closeMenu, { once: true });

  anchor?.setAttribute("aria-expanded", "true");

  session.cleanup = (): void => {
    clearSessionTimers(session);
    for (const panel of session.panels) panel.element.remove();
    session.panels.length = 0;
    anchor?.setAttribute("aria-expanded", "false");
    document.removeEventListener("mousedown", onOutside);
    document.removeEventListener("keydown", onEscape);
    document.removeEventListener("keydown", onMenuKeydown);
    window.removeEventListener("scroll", onWindowScroll, { capture: true });
    window.removeEventListener("resize", closeMenu);
    window.removeEventListener("blur", closeMenu);
    options.onClose?.();
  };

  // Radio-like menus such as priority focus the current value by default; others land on the first item.
  const checked = root.element.querySelector<HTMLButtonElement>('[aria-checked="true"]');
  (checked ?? root.entries[0]?.button)?.focus();
}

export function closeMenu(): void {
  if (!open) return;
  open.cleanup();
  open = null;
}

// ---------------------------------------------------------------------------
// Panels and menu items
// ---------------------------------------------------------------------------

function createPanel(
  session: OpenMenu,
  items: readonly MenuItem[],
  level: number,
  name: string,
  parent: Panel | null,
  parentEntry: PanelEntry | null,
  className?: string,
): Panel {
  const element = document.createElement("div");
  element.className = className ? `menu ${className}` : "menu";
  element.setAttribute("role", "menu");
  if (name) element.setAttribute("aria-label", name);
  element.style.zIndex = String(BASE_Z_INDEX + level);

  const panel: Panel = {
    element,
    level,
    name,
    parent,
    parentEntry,
    parentRow: parentEntry?.button ?? null,
    entries: [],
  };

  for (const item of items) {
    if (item.type === "separator") {
      const separator = document.createElement("div");
      separator.className = "menu-separator";
      separator.setAttribute("role", "separator");
      element.append(separator);
      continue;
    }
    if (item.type === "header") {
      const header = document.createElement("div");
      header.className = "menu-header";
      header.textContent = item.label;
      header.setAttribute("role", "presentation");
      element.append(header);
      continue;
    }

    const entry = createEntry(session, panel, item);
    panel.entries.push(entry);
    element.append(entry.button);
  }

  // When a panel scrolls, side panels hanging off it get misaligned — just close the deeper ones.
  element.addEventListener("scroll", () => {
    trimPanels(session, level + 1, true);
  });

  element.addEventListener("pointerenter", (event) => {
    if (event.pointerType !== "mouse" || open !== session) return;
    cancelIntent(session);
    cancelClose(session);
    cancelLeave(session);
  });

  element.addEventListener("pointerleave", (event) => {
    if (event.pointerType !== "mouse" || open !== session) return;
    scheduleLeave(session);
  });

  return panel;
}

function createEntry(session: OpenMenu, panel: Panel, item: MenuItem): PanelEntry {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "menu-item";
  button.setAttribute("role", item.checked === undefined ? "menuitem" : "menuitemradio");
  if (item.checked !== undefined) {
    button.setAttribute("aria-checked", String(item.checked));
  }

  const label = document.createElement("span");
  label.className = "menu-label";
  label.textContent = item.label;
  button.append(label);

  if (item.checked) {
    const check = document.createElement("span");
    check.className = "menu-check";
    check.textContent = "✓";
    button.append(check);
  }

  const branch = item.children !== undefined && item.children.length > 0;
  if (branch) {
    button.setAttribute("aria-haspopup", "menu");
    button.setAttribute("aria-expanded", "false");
    const caret = document.createElement("span");
    caret.className = "menu-caret";
    button.append(caret);
  }

  const entry: PanelEntry = { item, button, child: null };

  button.addEventListener("click", (event) => {
    if (open !== session) return;

    if (!branch) {
      const restore = session.restoreFocus ?? session.anchor;
      closeMenu();
      restore?.focus();
      item.onSelect?.();
      return;
    }

    cancelIntent(session);
    cancelClose(session);
    if (entry.child !== null) {
      trimPanels(session, panel.level + 1, true);
      return;
    }
    // A keyboard-triggered click has no mouse detail (detail === 0): move focus into the side panel as well.
    openSubmenu(session, panel, entry, { focusFirst: event.detail === 0 });
  });

  // Hovering this row: branches prepare to open a side panel; leaves only close stale sibling panels.
  button.addEventListener("pointerenter", (event) => {
    if (event.pointerType !== "mouse" || open !== session) return;
    cancelClose(session);
    cancelLeave(session);
    if (entry.child !== null) return;
    scheduleIntent(session, panel, entry);
  });

  button.addEventListener("pointerleave", (event) => {
    if (event.pointerType !== "mouse" || open !== session) return;
    if (session.intentEntry === entry) cancelIntent(session);
    if (entry.child !== null) scheduleClose(session, entry.child);
  });

  return entry;
}

// ---------------------------------------------------------------------------
// Opening and closing side panels
// ---------------------------------------------------------------------------

function openSubmenu(
  session: OpenMenu,
  panel: Panel,
  entry: PanelEntry,
  options: { focusFirst: boolean },
): void {
  if (open !== session || !session.panels.includes(panel)) return;
  const items = entry.item.children;
  if (items === undefined || items.length === 0) return;

  cancelIntent(session);
  cancelClose(session);
  cancelLeave(session);
  // Keep one path per level: close this panel's existing side panel first.
  trimPanels(session, panel.level + 1, true);

  const name = panel.name ? `${panel.name} / ${entry.item.label}` : entry.item.label;
  const child = createPanel(session, items, panel.level + 1, name, panel, entry);
  document.body.append(child.element);
  positionSubmenu(child.element, entry.button);
  session.panels.push(child);
  entry.child = child;
  entry.button.setAttribute("aria-expanded", "true");

  if (options.focusFirst) child.entries[0]?.button.focus();
}

/**
 * Close panels from the tail until only `keepCount` remain.
 * When focus is inside a panel being closed, hand it back to its parent row
 * (`focusFallback`).
 */
function trimPanels(session: OpenMenu, keepCount: number, focusFallback: boolean): void {
  if (open !== session) return;

  while (session.panels.length > keepCount) {
    const panel = session.panels[session.panels.length - 1];
    const focusInside =
      document.activeElement instanceof Node && panel.element.contains(document.activeElement);

    panel.element.remove();
    session.panels.pop();
    if (panel.parentEntry !== null) {
      panel.parentEntry.child = null;
      panel.parentEntry.button.setAttribute("aria-expanded", "false");
    }
    if (focusInside && focusFallback) panel.parentRow?.focus();
  }
}

function scheduleIntent(session: OpenMenu, panel: Panel, entry: PanelEntry): void {
  cancelIntent(session);
  session.intentEntry = entry;
  session.intentTimer = window.setTimeout(() => {
    session.intentTimer = null;
    session.intentEntry = null;
    if (open !== session) return;
    if (!entry.button.isConnected || !entry.button.matches(":hover")) return;

    // Already resting on another row: keep only this path on the right.
    trimPanels(session, panel.level + 1, true);
    if (entry.item.children !== undefined && entry.item.children.length > 0) {
      openSubmenu(session, panel, entry, { focusFirst: false });
    }
  }, HOVER_OPEN_DELAY);
}

function cancelIntent(session: OpenMenu): void {
  if (session.intentTimer !== null) window.clearTimeout(session.intentTimer);
  session.intentTimer = null;
  session.intentEntry = null;
}

function scheduleClose(session: OpenMenu, child: Panel): void {
  cancelClose(session);
  session.closeTimer = window.setTimeout(() => {
    session.closeTimer = null;
    if (open !== session || !child.element.isConnected) return;
    // Pointer came back to the parent row or side panel: keep it open.
    if (child.element.matches(":hover") || child.parentRow?.matches(":hover")) return;
    trimPanels(session, child.level, true);
  }, HOVER_CLOSE_DELAY);
}

function cancelClose(session: OpenMenu): void {
  if (session.closeTimer !== null) window.clearTimeout(session.closeTimer);
  session.closeTimer = null;
}

function scheduleLeave(session: OpenMenu): void {
  cancelLeave(session);
  session.leaveTimer = window.setTimeout(() => {
    session.leaveTimer = null;
    if (open !== session || anyPanelHovered()) return;
    // Pointer left the whole menu tree: close side panels only, keep the root menu open.
    trimPanels(session, 1, true);
    cancelIntent(session);
    cancelClose(session);
  }, LEAVE_DELAY);
}

function cancelLeave(session: OpenMenu): void {
  if (session.leaveTimer !== null) window.clearTimeout(session.leaveTimer);
  session.leaveTimer = null;
}

function clearSessionTimers(session: OpenMenu): void {
  cancelIntent(session);
  cancelClose(session);
  cancelLeave(session);
}

// ---------------------------------------------------------------------------
// Positioning
// ---------------------------------------------------------------------------

function placeRoot(element: HTMLElement, position: MenuPosition): void {
  const { offsetWidth: width, offsetHeight: height } = element;

  if (position.kind === "point") {
    const left =
      position.x + width > window.innerWidth ? window.innerWidth - width - EDGE : position.x;
    const top =
      position.y + height > window.innerHeight ? window.innerHeight - height - EDGE : position.y;
    element.style.left = `${Math.max(EDGE, Math.round(left))}px`;
    element.style.top = `${Math.max(EDGE, Math.round(top))}px`;
  } else {
    const rect = position.element.getBoundingClientRect();
    const gap = 4;
    const flipUp = rect.bottom + gap + height > window.innerHeight;
    const top = flipUp ? rect.top - gap - height : rect.bottom + gap;
    const left = Math.max(8, Math.min(rect.left, window.innerWidth - width - 8));
    element.style.left = `${Math.round(left)}px`;
    element.style.top = `${Math.max(EDGE, Math.round(top))}px`;
  }

  // The menu only grows downward past the viewport; clamp the scrollable height to the remaining space.
  const top = Number.parseFloat(element.style.top) || 0;
  element.style.maxHeight = `${Math.max(120, window.innerHeight - top - EDGE)}px`;
}

/**
 * The side panel hugs the parent row's right edge; if there is no room, it
 * flips to the left. It must be a separate fixed panel — the root menu has
 * overflow:auto, so nesting it inside would clip it.
 */
function positionSubmenu(element: HTMLElement, parentRow: HTMLElement): void {
  const rect = parentRow.getBoundingClientRect();
  const { offsetWidth: width, offsetHeight: height } = element;

  let left = rect.right + GAP;
  if (left + width > window.innerWidth - EDGE) left = rect.left - GAP - width;
  left = Math.max(EDGE, Math.min(left, window.innerWidth - width - EDGE));

  const top = Math.max(
    EDGE,
    Math.min(rect.top - MENU_PADDING, window.innerHeight - height - EDGE),
  );

  element.style.left = `${Math.round(left)}px`;
  element.style.top = `${Math.round(top)}px`;
}

// ---------------------------------------------------------------------------
// Global events
// ---------------------------------------------------------------------------

function onOutside(event: MouseEvent): void {
  if (!open) return;
  if (event.target instanceof Node) {
    if (menuContains(event.target)) return;
    // Let the anchor's own click handler deal with it: a second click should collapse, not "close then reopen".
    if (open.anchor?.contains(event.target)) return;
  }
  closeMenu();
}

/** Any page scroll closes the menu; scrolling inside a menu panel does not count. */
function onWindowScroll(event: Event): void {
  if (!open) return;
  if (event.target instanceof Node && menuContains(event.target)) return;
  closeMenu();
}

function onEscape(event: KeyboardEvent): void {
  if (event.key !== "Escape" || !open) return;
  event.preventDefault();
  const restore = open.restoreFocus;
  closeMenu();
  restore?.focus();
}

/**
 * Keyboard: up/down cycle within the current panel, right arrow enters a side
 * panel, left arrow returns to the parent panel. Side panels are separate
 * elements, so items must be picked from the "panel holding focus" rather than
 * flattening the whole tree.
 */
function onMenuKeydown(event: KeyboardEvent): void {
  if (!open) return;
  if (!["ArrowDown", "ArrowUp", "ArrowRight", "ArrowLeft"].includes(event.key)) return;
  if (!(event.target instanceof Node)) return;

  const panel = panelFor(event.target);
  if (panel === null || panel.entries.length === 0) return;

  const buttons = panel.entries.map((entry) => entry.button);
  const current =
    document.activeElement instanceof HTMLButtonElement &&
    buttons.includes(document.activeElement)
      ? document.activeElement
      : null;
  const index = current === null ? -1 : buttons.indexOf(current);

  switch (event.key) {
    case "ArrowDown": {
      event.preventDefault();
      buttons[(index + 1) % buttons.length].focus();
      break;
    }
    case "ArrowUp": {
      event.preventDefault();
      buttons[(index - 1 + buttons.length) % buttons.length].focus();
      break;
    }
    case "ArrowRight": {
      const entry = current === null ? null : panel.entries.find((item) => item.button === current);
      if (entry === null || entry === undefined) return;
      if (entry.item.children === undefined || entry.item.children.length === 0) return;
      event.preventDefault();
      if (entry.child !== null) entry.child.entries[0]?.button.focus();
      else openSubmenu(open, panel, entry, { focusFirst: true });
      break;
    }
    case "ArrowLeft": {
      if (panel.level === 0) return;
      event.preventDefault();
      const parentRow = panel.parentRow;
      trimPanels(open, panel.level, false);
      parentRow?.focus();
      break;
    }
  }
}

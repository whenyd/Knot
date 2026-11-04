/**
 * 通用弹出菜单：右键菜单、任务行「…」、优先级浮窗共用这一份实现。
 *
 * 只做“弹出、定位、关闭”三件事；菜单项的内容与行为由调用方给。
 * 定位支持指针点和元素锚定两种：前者用于右键，后者用于按钮下方。
 *
 * 带 children 的条目不再内联展开，而是在父行旁边弹出独立的侧向子面板：
 * 父面板的布局永远不变，收起子项时不会把指针下的目标整体拽走。
 */

export interface MenuItem {
  label: string;
  /** 叶子项的动作。带 `children` 的条目点击只打开/收起子面板，不会执行它。 */
  onSelect?: () => void;
  /**
   * 子项：条目变成可展开的分组，鼠标悬停、点击或键盘右键打开侧向子面板。
   * 可以嵌套，用来收拢"项目 → 模块 → 子模块"这类长列表。
   */
  children?: readonly MenuItem[];
  /** 当前选中项：显示勾，并让屏幕阅读器读作 menuitemradio。 */
  checked?: boolean;
}

export type MenuPosition =
  | { kind: "point"; x: number; y: number }
  | { kind: "anchor"; element: HTMLElement };

export interface MenuOptions {
  items: readonly MenuItem[];
  position: MenuPosition;
  /** 菜单的无障碍名，例如“优先级”。子面板会拼在它后面。 */
  label?: string;
  /** 关闭后把焦点还给谁；元素锚定时默认还给锚点。 */
  restoreFocus?: HTMLElement;
}

interface PanelEntry {
  item: MenuItem;
  button: HTMLButtonElement;
  /** 这一行打开的子面板；没打开为 null。 */
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
  /** 始终是一条路径：panels[i].parent === panels[i - 1]。 */
  panels: Panel[];
  anchor: HTMLElement | null;
  restoreFocus: HTMLElement | null;

  /** 悬停打开子面板的意图计时器。 */
  intentTimer: number | null;
  intentEntry: PanelEntry | null;
  /** 指针离开某一行后关闭它子面板的计时器。 */
  closeTimer: number | null;
  /** 指针离开整棵菜单树后的计时器。 */
  leaveTimer: number | null;

  cleanup: () => void;
}

let open: OpenMenu | null = null;

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

/** 事件目标落在哪一个面板里；键盘导航按当前面板取项。 */
function panelFor(node: Node): Panel | null {
  if (!open) return null;
  for (let index = open.panels.length - 1; index >= 0; index -= 1) {
    if (open.panels[index].element.contains(node)) return open.panels[index];
  }
  return null;
}

/**
 * 弹出菜单。`items` 为空就什么也不做——“这个位置没有可做的事”不该弹一个空框。
 */
export function openMenu(options: MenuOptions): void {
  const { items, position } = options;
  const anchor = position.kind === "anchor" ? position.element : null;

  // 同一个锚点再点一次 = 收起（按钮语义）。
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

  const root = createPanel(session, items, 0, options.label ?? "", null, null);
  document.body.append(root.element);
  session.panels.push(root);
  placeRoot(root.element, position);

  // 面板事件都挂在各自面板上；全局只留一份。
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
  };

  // 优先级这类单选项默认把焦点放进当前值，其余菜单落在第一项。
  const checked = root.element.querySelector<HTMLButtonElement>('[aria-checked="true"]');
  (checked ?? root.entries[0]?.button)?.focus();
}

export function closeMenu(): void {
  if (!open) return;
  open.cleanup();
  open = null;
}

// ---------------------------------------------------------------------------
// 面板与菜单项
// ---------------------------------------------------------------------------

function createPanel(
  session: OpenMenu,
  items: readonly MenuItem[],
  level: number,
  name: string,
  parent: Panel | null,
  parentEntry: PanelEntry | null,
): Panel {
  const element = document.createElement("div");
  element.className = "menu";
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
    const entry = createEntry(session, panel, item);
    panel.entries.push(entry);
    element.append(entry.button);
  }

  // 面板自己滚动时，挂在它上面的子面板会错位——直接关掉更深层。
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
    // 键盘触发的 click 没有鼠标细节（detail === 0）：顺手把焦点送进子面板。
    openSubmenu(session, panel, entry, { focusFirst: event.detail === 0 });
  });

  // 悬停到本行：分支准备打开子面板，叶子只负责关掉同级的旧子面板。
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
// 子面板的打开与关闭
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
  // 同一层只保留一条路径：先关掉这个面板已有的子面板。
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
 * 从尾部关掉面板，直到只剩 `keepCount` 个。
 * 焦点在将被关掉的面板里时，交还给它的父行（`focusFallback`）。
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

    // 已经停在别的行上：右侧只留这一条路径。
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
    // 指针又回到父行或子面板里：保持打开。
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
    // 指针离开整棵菜单树：只收子面板，根菜单保持打开。
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
// 定位
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

  // 菜单只会往下长到视口外；把可滚动高度收紧到剩余空间。
  const top = Number.parseFloat(element.style.top) || 0;
  element.style.maxHeight = `${Math.max(120, window.innerHeight - top - EDGE)}px`;
}

/**
 * 子面板贴在父行右侧；右边放不下就翻到左侧。
 * 必须是独立的 fixed 面板——根菜单有 overflow:auto，放在里面会被裁掉。
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
// 全局事件
// ---------------------------------------------------------------------------

function onOutside(event: MouseEvent): void {
  if (!open) return;
  if (event.target instanceof Node) {
    if (menuContains(event.target)) return;
    // 点锚点交给它自己的 click 处理：再点一次应当是收起，而不是“关掉再打开”。
    if (open.anchor?.contains(event.target)) return;
  }
  closeMenu();
}

/** 页面滚动就收起菜单；菜单面板自己内部滚不算。 */
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
 * 键盘：上下在当前面板内循环，右键进入子面板，左键退回父面板。
 * 子面板是独立元素，因此必须按“焦点所在面板”取项，不能再把整棵树拍平。
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

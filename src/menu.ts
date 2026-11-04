/**
 * 通用弹出菜单：右键菜单、任务行「…」、优先级浮窗共用这一份实现。
 *
 * 只做“弹出、定位、关闭”三件事；菜单项的内容与行为由调用方给。
 * 定位支持指针点和元素锚定两种：前者用于右键，后者用于按钮下方。
 */

export interface MenuItem {
  label: string;
  onSelect: () => void;
  /** 左侧色点（优先级用）。传 CSS 颜色，不传就不画。 */
  dot?: string;
  /** 当前选中项：显示勾，并让屏幕阅读器读作 menuitemradio。 */
  checked?: boolean;
}

export type MenuPosition =
  | { kind: "point"; x: number; y: number }
  | { kind: "anchor"; element: HTMLElement };

export interface MenuOptions {
  items: readonly MenuItem[];
  position: MenuPosition;
  /** 菜单的无障碍名，例如“优先级”。 */
  label?: string;
  /** 关闭后把焦点还给谁；元素锚定时默认还给锚点。 */
  restoreFocus?: HTMLElement;
}

interface OpenMenu {
  element: HTMLElement;
  anchor: HTMLElement | null;
  restoreFocus: HTMLElement | null;
  cleanup: () => void;
}

let open: OpenMenu | null = null;

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

  const element = document.createElement("div");
  element.className = "menu";
  element.setAttribute("role", "menu");
  if (options.label) element.setAttribute("aria-label", options.label);

  const buttons = items.map((item) => {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "menu-item";
    button.setAttribute("role", item.checked === undefined ? "menuitem" : "menuitemradio");
    if (item.checked !== undefined) {
      button.setAttribute("aria-checked", String(item.checked));
    }

    if (item.dot !== undefined) {
      const dot = document.createElement("span");
      dot.className = "menu-dot";
      dot.style.background = item.dot;
      button.append(dot);
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

    button.addEventListener("click", () => {
      const restore = open?.restoreFocus ?? open?.anchor ?? null;
      closeMenu();
      restore?.focus();
      item.onSelect();
    });

    element.append(button);
    return button;
  });

  // 挂在 body 上：左栏和表格都有 overflow，挂在行里会被裁掉。
  document.body.append(element);

  const { offsetWidth: width, offsetHeight: height } = element;
  if (position.kind === "point") {
    const left = position.x + width > window.innerWidth ? window.innerWidth - width - 4 : position.x;
    const top = position.y + height > window.innerHeight ? window.innerHeight - height - 4 : position.y;
    element.style.left = `${Math.max(4, Math.round(left))}px`;
    element.style.top = `${Math.max(4, Math.round(top))}px`;
  } else {
    const rect = position.element.getBoundingClientRect();
    const gap = 4;
    const flipUp = rect.bottom + gap + height > window.innerHeight;
    const top = flipUp ? rect.top - gap - height : rect.bottom + gap;
    const left = Math.max(8, Math.min(rect.left, window.innerWidth - width - 8));
    element.style.left = `${Math.round(left)}px`;
    element.style.top = `${Math.max(4, Math.round(top))}px`;
  }

  // 优先级这类单选项默认把焦点放进当前值，其余菜单落在第一项。
  (element.querySelector<HTMLButtonElement>('[aria-checked="true"]') ?? buttons[0])?.focus();
  element.addEventListener("keydown", onMenuKeydown);

  anchor?.setAttribute("aria-expanded", "true");

  document.addEventListener("mousedown", onOutside);
  document.addEventListener("keydown", onEscape);
  window.addEventListener("scroll", closeMenu, { capture: true, once: true });
  window.addEventListener("resize", closeMenu, { once: true });
  window.addEventListener("blur", closeMenu, { once: true });

  open = {
    element,
    anchor,
    restoreFocus: options.restoreFocus ?? anchor,
    cleanup: () => {
      element.remove();
      anchor?.setAttribute("aria-expanded", "false");
      document.removeEventListener("mousedown", onOutside);
      document.removeEventListener("keydown", onEscape);
      window.removeEventListener("scroll", closeMenu, { capture: true });
      window.removeEventListener("resize", closeMenu);
      window.removeEventListener("blur", closeMenu);
    },
  };
}

export function closeMenu(): void {
  if (!open) return;
  open.cleanup();
  open = null;
}

function onOutside(event: MouseEvent): void {
  if (!open) return;
  if (event.target instanceof Node) {
    if (open.element.contains(event.target)) return;
    // 点锚点交给它自己的 click 处理：再点一次应当是收起，而不是“关掉再打开”。
    if (open.anchor?.contains(event.target)) return;
  }
  closeMenu();
}

function onEscape(event: KeyboardEvent): void {
  if (event.key !== "Escape") return;
  event.preventDefault();
  const restore = open?.restoreFocus ?? null;
  closeMenu();
  restore?.focus();
}

/** 上下键在菜单项之间移动。 */
function onMenuKeydown(event: KeyboardEvent): void {
  if (!open) return;
  if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;

  const items = [...open.element.querySelectorAll<HTMLButtonElement>(".menu-item")];
  if (items.length === 0) return;
  event.preventDefault();

  const index = items.indexOf(document.activeElement as HTMLButtonElement);
  const step = event.key === "ArrowDown" ? 1 : -1;
  items[(index + step + items.length) % items.length].focus();
}

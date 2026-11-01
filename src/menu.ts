/**
 * 左栏右键菜单：一个通用组件，谁要给谁。
 *
 * 只做"弹出、定位、关闭"三件事；菜单项的内容与行为由调用方给。
 * 与优先级浮窗是两套实现（那边要带圆点和勾），T7 把它们并成一份。
 */

export interface MenuItem {
  label: string;
  onSelect: () => void;
}

let open: { element: HTMLElement; cleanup: () => void } | null = null;

/**
 * 在指针位置弹出一个菜单。
 *
 * `items` 为空就什么也不做——"这个位置没有可做的事"不该弹一个空框。
 */
export function openMenu(x: number, y: number, items: readonly MenuItem[]): void {
  closeMenu();
  if (items.length === 0) return;

  const element = document.createElement("div");
  element.className = "menu";
  element.setAttribute("role", "menu");

  const buttons = items.map((item) => {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "menu-item";
    button.setAttribute("role", "menuitem");
    button.textContent = item.label;
    button.addEventListener("click", () => {
      closeMenu();
      item.onSelect();
    });
    element.append(button);
    return button;
  });

  // 挂在 body 上：左栏有 overflow，挂在行里会被裁掉。
  document.body.append(element);

  // 贴指针，但不许跑出视口
  const { offsetWidth: width, offsetHeight: height } = element;
  const left = x + width > window.innerWidth ? window.innerWidth - width - 4 : x;
  const top = y + height > window.innerHeight ? window.innerHeight - height - 4 : y;
  element.style.left = `${Math.max(4, Math.round(left))}px`;
  element.style.top = `${Math.max(4, Math.round(top))}px`;

  buttons[0]?.focus();
  element.addEventListener("keydown", onMenuKeydown);

  document.addEventListener("mousedown", onOutside);
  document.addEventListener("keydown", onEscape);
  window.addEventListener("scroll", closeMenu, { capture: true, once: true });
  window.addEventListener("resize", closeMenu, { once: true });
  window.addEventListener("blur", closeMenu, { once: true });

  open = {
    element,
    cleanup: () => {
      element.remove();
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
  if (open && event.target instanceof Node && open.element.contains(event.target)) return;
  closeMenu();
}

function onEscape(event: KeyboardEvent): void {
  if (event.key === "Escape") {
    event.preventDefault();
    closeMenu();
  }
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

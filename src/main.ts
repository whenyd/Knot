/**
 * 视图层：状态 + 事件 + 渲染。原生 DOM，没有 UI 框架。
 *
 * 数据只朝一个方向流：
 *   `list_projects` / `list_modules` / `list_tasks`  →  纯函数排序分组（grouping.ts）  →  DOM
 * 没有任何反向绑定：界面永远是"重取一遍再全量重画"。数据量小，重画的代价
 * 远低于维护增量 patch 的代价。
 */
import {
  api,
  type Container,
  type Module,
  type MoveTarget,
  type NodeKind,
  type Priority,
  type Project,
  type Status,
  type Task,
} from "./api";
import { closeMenu, openMenu, type MenuItem, type MenuPosition } from "./menu";
import {
  buildGroups,
  countGroup,
  countGroups,
  filterTasks,
  resolveTarget,
  scopeLabel,
  selectScope,
  sortTasks,
  taskMatchesFilters,
  type Group,
  type Scope,
  type TaskFilters,
} from "./grouping";
import { initTooltips } from "./tooltip";
import appIcon from "../src-tauri/icons/icon.png";

/** 数据库起不来时，后端通过这个 query 参数把原因交给前端。 */
const STARTUP_ERROR_PARAM = "startupError";

/** 与 src-tauri/tauri.conf.json 的 version 保持一致。 */
const APP_VERSION = "0.1.0";

// 状态是三态**循环**：一个字段一个控件，点一下换下一个值。
const NEXT_STATUS: Record<Status, Status> = { todo: "doing", doing: "done", done: "todo" };
const STATUS_LABELS: Record<Status, string> = { todo: "未完成", doing: "进行中", done: "已完成" };
const STATUS_ORDER: Status[] = ["todo", "doing", "done"];

// 优先级是浮窗选择：三档都摆出来，不用循环着找。
const PRIORITY_ORDER: Priority[] = ["high", "mid", "low"];
const PRIORITY_LABELS: Record<Priority, string> = { high: "高", mid: "中", low: "低" };

/** 三列时间共用一套文案：创建时间在新建时写入，开始 / 结束随状态变化。 */
type TimeField = "created" | "start" | "end";
const TIME_LABELS: Record<TimeField, string> = { created: "创建", start: "开始", end: "结束" };
const TIME_HINTS: Record<TimeField, string> = {
  created: "创建任务时自动记录",
  start: "随状态自动记录",
  end: "随状态自动记录",
};

/** `2026-11-04T09:00` → `2026-11-04 09:00`；未设置显示破折号。 */
function formatSchedule(value: string | null): string {
  return value === null ? "—" : value.replace("T", " ");
}

interface State {
  projects: Project[];
  container: Container;
  /** 当前项目上下文；切到「我的待办」后仍保留，用于左栏项目名和模块树。 */
  projectId: number | null;
  scope: Scope;
  modules: Module[];
  tasks: Task[];
  /** 当前项目上下文的全部任务，供左栏模块树计数；与表格容器解耦。 */
  projectTasks: Task[];
  /** 左栏范围树里**收起**的模块 id。默认展开，所以存的是收起的那几个。 */
  collapsedNodes: Set<number>;
  /** 表格里收起的分类分组，key 由 `groupKey` 生成。与左栏的收起互不影响。 */
  collapsedGroups: Set<string>;
  /** 正在内联添加子任务的父任务 id；同一时刻只有一个。 */
  addingSubtaskFor: number | null;
  /** 表格里收起了子任务的父任务 id。默认展开，所以存的是收起的那几个。 */
  collapsedTasks: Set<number>;
  /** 顶部筛选在本次会话内保留，切换范围或项目时不重置。 */
  statusFilter: Set<Status>;
  priorityFilter: Set<Priority>;
}

const state: State = {
  projects: [],
  container: { kind: "todo" },
  projectId: null,
  scope: { kind: "project" },
  modules: [],
  tasks: [],
  projectTasks: [],
  collapsedNodes: new Set(),
  collapsedGroups: new Set(),
  addingSubtaskFor: null,
  collapsedTasks: new Set(),
  statusFilter: new Set(["todo", "doing"]),
  priorityFilter: new Set(PRIORITY_ORDER),
};

/** 内联子任务输入框的草稿：放在状态对象外，无关重绘也不丢正在打的字。 */
let subtaskDraft = "";
/** 输入会话号：旧的失焦回调只认自己那一轮，免得点「＋」时误提交上一轮。 */
let subtaskInputSession = 0;
/** 首次加载时把上次项目接回来；之后的 reload 不能再覆盖用户当前选择。 */
let startupProjectRestored = false;

// ---------------------------------------------------------------------------
// DOM
// ---------------------------------------------------------------------------

function must<T extends HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (!node) throw new Error(`缺少 DOM 节点 #${id}`);
  return node as T;
}

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className?: string,
  text?: string,
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

const SVG_NS = "http://www.w3.org/2000/svg";

/** 引用 index.html 里的 symbol；尺寸交给 CSS。 */
function icon(name: string, className = "icon"): SVGSVGElement {
  const svg = document.createElementNS(SVG_NS, "svg");
  svg.setAttribute("class", className);
  svg.setAttribute("viewBox", "0 0 16 16");
  svg.setAttribute("aria-hidden", "true");
  const use = document.createElementNS(SVG_NS, "use");
  use.setAttribute("href", `#i-${name}`);
  svg.append(use);
  return svg;
}

const dom = {
  todoSection: must<HTMLElement>("todo-section"),
  projectHeading: must<HTMLElement>("project-heading"),
  scopeSection: must<HTMLElement>("scope-section"),
  projectBtn: must<HTMLButtonElement>("project-btn"),
  filters: must<HTMLElement>("filters"),
  titlebarDrag: must<HTMLElement>("titlebar-drag"),
  windowMin: must<HTMLButtonElement>("window-min"),
  windowMax: must<HTMLButtonElement>("window-max"),
  windowClose: must<HTMLButtonElement>("window-close"),
  grid: must<HTMLTableElement>("grid"),
  body: must<HTMLTableSectionElement>("grid-body"),
  empty: must<HTMLParagraphElement>("empty"),
  taskForm: must<HTMLFormElement>("task-form"),
  taskInput: must<HTMLInputElement>("task-input"),
  statusCount: must<HTMLElement>("status-count"),
  themeBtn: must<HTMLButtonElement>("theme-btn"),
  themeLabel: must<HTMLElement>("theme-label"),
  aboutBtn: must<HTMLButtonElement>("about-btn"),
  aboutModal: must<HTMLElement>("about-modal"),
  aboutOk: must<HTMLButtonElement>("about-ok"),
  aboutIcon: must<HTMLImageElement>("about-icon"),
  aboutVersion: must<HTMLElement>("about-version"),
  aboutVersionRow: must<HTMLElement>("about-version-row"),
  aboutPlatform: must<HTMLElement>("about-platform"),
  aboutRuntime: must<HTMLElement>("about-runtime"),
  notice: must<HTMLElement>("notice"),
  resizeTip: must<HTMLElement>("resize-tip"),
  modal: must<HTMLElement>("modal"),
  modalCard: must<HTMLElement>("modal-card"),
  modalTitle: must<HTMLElement>("modal-title"),
  modalBody: must<HTMLElement>("modal-body"),
  modalField: must<HTMLElement>("modal-field"),
  modalLabel: must<HTMLElement>("modal-label"),
  modalInput: must<HTMLInputElement>("modal-input"),
  modalTextarea: must<HTMLTextAreaElement>("modal-textarea"),
  modalOk: must<HTMLButtonElement>("modal-ok"),
  modalCancel: must<HTMLButtonElement>("modal-cancel"),
  devReset: must<HTMLButtonElement>("dev-reset"),
};

// 删库重建只在开发版出现；正式构建里整段分支会被裁掉，按钮保持 hidden。
if (import.meta.env.DEV) {
  dom.devReset.hidden = false;
  dom.devReset.addEventListener("click", () => void resetDatabaseFlow());
}

// ---------------------------------------------------------------------------
// 主题与当前项目
// ---------------------------------------------------------------------------

type ThemeMode = "light" | "dark" | "system";
const THEME_STORAGE_KEY = "knot.theme.v1";
const PROJECT_STORAGE_KEY = "knot.currentProject.v1";

function readThemeMode(): ThemeMode {
  try {
    const stored = localStorage.getItem(THEME_STORAGE_KEY);
    return stored === "light" || stored === "dark" || stored === "system" ? stored : "light";
  } catch {
    return "light";
  }
}

function applyTheme(mode: ThemeMode): void {
  const prefersDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
  const resolved = mode === "system" ? (prefersDark ? "dark" : "light") : mode;
  document.documentElement.dataset.theme = resolved;
  try {
    localStorage.setItem(THEME_STORAGE_KEY, mode);
  } catch {
    // 存储不可用只影响下次启动的偏好，不阻塞主题切换。
  }

  const iconName = mode === "light" ? "sun" : mode === "dark" ? "moon" : "monitor";
  dom.themeBtn.querySelector("use")?.setAttribute("href", `#i-${iconName}`);
  dom.themeLabel.textContent = mode === "light" ? "浅色" : mode === "dark" ? "深色" : "跟随系统";
}

function initTheme(): void {
  applyTheme(readThemeMode());
  dom.themeBtn.addEventListener("click", () => {
    const mode = readThemeMode();
    openMenu({
      items: [
        { label: "浅色", checked: mode === "light", onSelect: () => applyTheme("light") },
        { label: "深色", checked: mode === "dark", onSelect: () => applyTheme("dark") },
        { label: "跟随系统", checked: mode === "system", onSelect: () => applyTheme("system") },
      ],
      position: { kind: "anchor", element: dom.themeBtn },
      label: "主题",
    });
  });
  window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
    if (readThemeMode() === "system") applyTheme("system");
  });
}

type AppPlatform = "windows" | "macos" | "linux";

/** macOS 用原生红绿灯；Windows 用顶栏里的自绘按钮；Linux 保留系统标题栏。 */
function detectPlatform(): AppPlatform {
  const ua = navigator.userAgent;
  if (/Macintosh|Mac OS X/i.test(ua)) return "macos";
  if (/Linux/i.test(ua) && !/Android/i.test(ua)) return "linux";
  return "windows";
}

async function initWindowControls(): Promise<void> {
  const platform = detectPlatform();
  const isTauri = "__TAURI_INTERNALS__" in window;
  document.documentElement.dataset.platform = platform;
  document.documentElement.dataset.tauri = isTauri ? "true" : "false";
  if (platform !== "windows" || !isTauri) return;

  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    const appWindow = getCurrentWindow();
    const maxIcon = dom.windowMax.querySelector("use");

    const syncMaximized = async () => {
      const maximized = await appWindow.isMaximized();
      maxIcon?.setAttribute("href", maximized ? "#i-restore" : "#i-maximize");
      const label = maximized ? "还原" : "最大化";
      dom.windowMax.title = label;
      dom.windowMax.setAttribute("aria-label", label);
    };

    dom.windowMin.addEventListener("click", () => void appWindow.minimize());
    dom.windowMax.addEventListener("click", () => void appWindow.toggleMaximize());
    dom.windowClose.addEventListener("click", () => void appWindow.close());
    dom.titlebarDrag.addEventListener("dblclick", () => void appWindow.toggleMaximize());

    await syncMaximized();
    await appWindow.onResized(() => {
      void syncMaximized();
    });
  } catch (error) {
    // 浏览器里跑 Vite 时没有 Tauri 运行时，按钮只作为静态预览。
    console.warn("窗口控制初始化失败", error);
  }
}

function platformLabel(): string {
  const platform = detectPlatform();
  const os = platform === "macos" ? "macOS" : platform === "linux" ? "Linux" : "Windows";
  const ua = navigator.userAgent;
  const arch = /arm64|aarch64/i.test(ua)
    ? "ARM64"
    : /WOW64|Win32/i.test(ua) && !/x64/i.test(ua)
      ? "x86"
      : "x64";
  return `${os} · ${arch}`;
}

function runtimeLabel(): string {
  const platform = detectPlatform();
  if (platform === "macos") return "Tauri 2 · WKWebView";
  if (platform === "linux") return "Tauri 2 · WebKitGTK";
  return "Tauri 2 · WebView2";
}

function openAboutDialog(): void {
  if (!dom.aboutModal.hidden) return;
  closeMenu();
  dom.aboutIcon.src = appIcon;
  dom.aboutVersion.textContent = `v${APP_VERSION}`;
  dom.aboutVersionRow.textContent = APP_VERSION;
  dom.aboutPlatform.textContent = platformLabel();
  dom.aboutRuntime.textContent = runtimeLabel();
  dom.aboutModal.hidden = false;
  dom.aboutOk.focus();

  const close = (): void => {
    dom.aboutModal.hidden = true;
    dom.aboutOk.removeEventListener("click", close);
    dom.aboutModal.removeEventListener("click", onBackdrop);
    document.removeEventListener("keydown", onKey);
    dom.aboutBtn.focus();
  };
  const onBackdrop = (event: MouseEvent): void => {
    if (event.target === dom.aboutModal) close();
  };
  const onKey = (event: KeyboardEvent): void => {
    if (event.key === "Escape") close();
  };

  dom.aboutOk.addEventListener("click", close);
  dom.aboutModal.addEventListener("click", onBackdrop);
  document.addEventListener("keydown", onKey);
}

function readSavedProjectId(): number | null {
  try {
    const raw = localStorage.getItem(PROJECT_STORAGE_KEY);
    if (raw === null) return null;
    const id = Number(raw);
    return Number.isInteger(id) && id > 0 ? id : null;
  } catch {
    return null;
  }
}

/** 记住当前项目上下文；「我的待办」不是项目，不写入。 */
function persistCurrentProject(): void {
  try {
    if (state.projectId !== null) {
      localStorage.setItem(PROJECT_STORAGE_KEY, String(state.projectId));
    } else {
      localStorage.removeItem(PROJECT_STORAGE_KEY);
    }
  } catch {
    // 存储不可用时退回本次会话的导航行为。
  }
}

function deleteCurrentProject(): void {
  const project = currentProject();
  if (!project || project.isTodo) {
    showNotice("「我的待办」不是项目，不能删除");
    return;
  }
  void deleteProjectFlow(project);
}

function openProjectMenu(): void {
  const items: MenuItem[] = [
    { label: "新建项目…", onSelect: () => void createProjectFlow() },
    { label: "删除项目", onSelect: deleteCurrentProject },
    { type: "separator", label: "" },
    { type: "header", label: "所有项目" },
    ...state.projects
      .filter((project) => !project.isTodo)
      .map((project) => ({
        label: project.name,
        checked: isCurrent(project),
        onSelect: () => void selectProject(project),
      })),
  ];

  openMenu({
    items,
    position: { kind: "anchor", element: dom.projectBtn },
    label: "项目",
    className: "is-project",
  });
}

// ---------------------------------------------------------------------------
// 列宽
// ---------------------------------------------------------------------------

type ResizableColumn = "status" | "title" | "created" | "start" | "end" | "priority" | "note";
type FixedColumn = Exclude<ResizableColumn, "title">;

/** 「任务」列吃掉剩余宽度，其余列的宽度由用户拖动或默认值决定。 */
const FIXED_COLUMNS: readonly FixedColumn[] = [
  "status",
  "priority",
  "created",
  "start",
  "end",
  "note",
];
const COLUMN_MIN: Record<ResizableColumn, number> = {
  status: 36,
  title: 100,
  created: 80,
  start: 80,
  end: 80,
  priority: 48,
  note: 60,
};
const COLUMN_LABELS: Record<ResizableColumn, string> = {
  status: "状态",
  title: "任务",
  created: "创建",
  start: "开始",
  end: "结束",
  priority: "优先级",
  note: "备注",
};
/** 与 index.html 的 col / th 顺序保持一致，拖动时按这个顺序向两侧找空间。 */
const COLUMN_ORDER: readonly ResizableColumn[] = [
  "status",
  "title",
  "priority",
  "created",
  "start",
  "end",
  "note",
];
const COLUMN_STORAGE_KEY = "knot.columnWidths.v1";

let fixedWidths: Record<FixedColumn, number> | null = null;
let titleColumnWidth = 0;

function columnElement(key: ResizableColumn): HTMLTableColElement | null {
  return dom.grid.querySelector<HTMLTableColElement>(`col[data-col="${key}"]`);
}

function defaultFixedWidths(): Record<FixedColumn, number> {
  const out = {} as Record<FixedColumn, number>;
  for (const key of FIXED_COLUMNS) {
    const parsed = Number.parseFloat(columnElement(key)?.style.width ?? "");
    out[key] = Number.isFinite(parsed) ? parsed : COLUMN_MIN[key];
  }
  return out;
}

function loadFixedWidths(): Partial<Record<FixedColumn, number>> {
  try {
    const raw = localStorage.getItem(COLUMN_STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as Record<string, unknown>;
    const out: Partial<Record<FixedColumn, number>> = {};
    for (const key of FIXED_COLUMNS) {
      const value = parsed[key];
      if (typeof value === "number" && Number.isFinite(value)) {
        out[key] = Math.max(COLUMN_MIN[key], value);
      }
    }
    return out;
  } catch {
    // 存储不可用或内容坏了：回退到默认列宽，不影响本次使用。
    return {};
  }
}

function currentFixedWidths(): Record<FixedColumn, number> {
  if (fixedWidths !== null) return fixedWidths;
  const widths = defaultFixedWidths();
  const stored = loadFixedWidths();
  for (const key of FIXED_COLUMNS) {
    if (stored[key] !== undefined) widths[key] = stored[key];
  }
  fixedWidths = widths;
  return widths;
}

/** 固定列按当前宽度落位，「任务」列补满剩余空间；窗口变窄时任务列不低于最小值。 */
function applyColumnWidths(): void {
  const widths = currentFixedWidths();
  const fixedTotal = FIXED_COLUMNS.reduce((sum, key) => sum + widths[key], 0);
  titleColumnWidth = Math.max(COLUMN_MIN.title, dom.grid.clientWidth - fixedTotal);

  for (const key of FIXED_COLUMNS) {
    const col = columnElement(key);
    if (col) col.style.width = `${widths[key]}px`;
  }
  const titleCol = columnElement("title");
  if (titleCol) titleCol.style.width = `${titleColumnWidth}px`;
}

function saveColumnWidths(): void {
  try {
    localStorage.setItem(COLUMN_STORAGE_KEY, JSON.stringify(currentFixedWidths()));
  } catch {
    // 只影响“下次还记得”，本次拖动已经生效。
  }
}

function columnWidthOf(key: ResizableColumn, widths: Record<FixedColumn, number>): number {
  return key === "title" ? titleColumnWidth : widths[key as FixedColumn];
}

/** 当前所有列的显示宽度，拖动开始时取一次快照，后续位移都基于它计算。 */
function columnWidthsSnapshot(): Record<ResizableColumn, number> {
  const fixed = currentFixedWidths();
  const out = {} as Record<ResizableColumn, number>;
  for (const key of COLUMN_ORDER) out[key] = columnWidthOf(key, fixed);
  return out;
}

function applyWidths(widths: Record<ResizableColumn, number>): void {
  for (const key of COLUMN_ORDER) {
    const col = columnElement(key);
    if (col) col.style.width = `${widths[key]}px`;
  }
  titleColumnWidth = widths.title;
}

/**
 * 把一次水平位移分给分界线两侧：
 * - 向右拖：线右侧从近到远依次让出空间，直到最小宽度；左侧只增加右侧真正让出的量。
 * - 向左拖：线左侧从近到远依次让出空间；右侧只增加左侧真正让出的量。
 * 这样某一侧已经到最小宽度时，会自然带动同侧更远的列，而不是把线卡死。
 */
function cascadeWidths(
  start: Record<ResizableColumn, number>,
  boundary: number,
  delta: number,
): { widths: Record<ResizableColumn, number>; applied: number } {
  const widths = { ...start };
  let remaining = Math.abs(delta);

  if (delta > 0) {
    for (let i = boundary; i < COLUMN_ORDER.length && remaining > 0; i += 1) {
      const key = COLUMN_ORDER[i];
      const take = Math.min(widths[key] - COLUMN_MIN[key], remaining);
      if (take <= 0) continue;
      widths[key] -= take;
      remaining -= take;
    }
  } else if (delta < 0) {
    for (let i = boundary - 1; i >= 0 && remaining > 0; i -= 1) {
      const key = COLUMN_ORDER[i];
      const take = Math.min(widths[key] - COLUMN_MIN[key], remaining);
      if (take <= 0) continue;
      widths[key] -= take;
      remaining -= take;
    }
  }

  const applied = Math.abs(delta) - remaining;
  if (applied > 0) {
    if (delta > 0) widths[COLUMN_ORDER[boundary - 1]] += applied;
    else widths[COLUMN_ORDER[boundary]] += applied;
  }
  return { widths, applied };
}

function columnHeader(key: ResizableColumn): HTMLElement | null {
  return dom.grid.querySelector<HTMLElement>(`th[data-col="${key}"]`);
}

function highlightColumns(left: ResizableColumn, right: ResizableColumn, on: boolean): void {
  for (const key of [left, right]) {
    const header = columnHeader(key);
    header?.classList.toggle("is-resizing", on);
    if (on) header?.classList.remove("is-resize-neighbor");
  }
}

function showResizeTip(
  event: PointerEvent,
  left: ResizableColumn,
  right: ResizableColumn,
  widths: Record<ResizableColumn, number>,
): void {
  const leftWidth = Math.round(widths[left]);
  const rightWidth = Math.round(widths[right]);
  dom.resizeTip.textContent = `${COLUMN_LABELS[left]} ${leftWidth}px · ${COLUMN_LABELS[right]} ${rightWidth}px`;
  dom.resizeTip.hidden = false;
  const leftPosition = Math.min(
    window.innerWidth - dom.resizeTip.offsetWidth - 8,
    event.clientX + 14,
  );
  dom.resizeTip.style.left = `${Math.max(8, leftPosition)}px`;
  dom.resizeTip.style.top = `${Math.min(window.innerHeight - 36, event.clientY + 20)}px`;
}

function beginColumnResize(
  event: PointerEvent,
  left: ResizableColumn,
  right: ResizableColumn,
): void {
  if (event.button !== 0) return;
  event.preventDefault();
  event.stopPropagation();

  const startWidths = columnWidthsSnapshot();
  const boundary = COLUMN_ORDER.indexOf(right);
  if (boundary <= 0 || COLUMN_ORDER[boundary - 1] !== left) return;

  const startX = event.clientX;
  let lastWidths = startWidths;

  const onMove = (moveEvent: PointerEvent): void => {
    const { widths } = cascadeWidths(startWidths, boundary, moveEvent.clientX - startX);
    lastWidths = widths;
    applyWidths(widths);
    showResizeTip(moveEvent, left, right, widths);
  };

  const finish = (): void => {
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", finish);
    window.removeEventListener("pointercancel", finish);
    document.body.classList.remove("col-resizing");
    dom.resizeTip.hidden = true;
    highlightColumns(left, right, false);

    const fixed = currentFixedWidths();
    for (const key of FIXED_COLUMNS) fixed[key] = lastWidths[key];
    applyColumnWidths();
    saveColumnWidths();
  };

  document.body.classList.add("col-resizing");
  highlightColumns(left, right, true);
  showResizeTip(event, left, right, startWidths);
  window.addEventListener("pointermove", onMove);
  window.addEventListener("pointerup", finish);
  window.addEventListener("pointercancel", finish);
}

/**
 * 表头每一条分界线都是拖动把手：拖动时同时改变线两侧的列，总宽度不变；
 * 一侧到最小宽度后继续拖动，会按列顺序带动同侧更远的列。
 * 悬停时两侧表头一起变色，拖动时浮出两列当前宽度，避免“只动了其中一列”的错觉。
 */
function initColumnResizing(): void {
  applyColumnWidths();
  for (const handle of dom.grid.querySelectorAll<HTMLElement>(".col-resizer")) {
    const boundary = handle.dataset.boundary?.split(":");
    if (!boundary || boundary.length !== 2) continue;
    const [left, right] = boundary as [ResizableColumn, ResizableColumn];
    handle.title = "拖动调整列宽（到最小后带动相邻列）";
    handle.addEventListener("pointerdown", (event) => beginColumnResize(event, left, right));

    const highlight = (on: boolean): void => {
      if (document.body.classList.contains("col-resizing")) return;
      for (const key of [left, right]) {
        columnHeader(key)?.classList.toggle("is-resize-neighbor", on);
      }
    };
    handle.addEventListener("pointerenter", () => highlight(true));
    handle.addEventListener("pointerleave", () => highlight(false));
  }
  window.addEventListener("resize", applyColumnWidths);
  window.requestAnimationFrame(applyColumnWidths);
}

// ---------------------------------------------------------------------------
// 加载
// ---------------------------------------------------------------------------

/** 取回当前容器的全部数据并重画。所有写操作之后都调它。 */
async function reload(): Promise<void> {
  try {
    state.projects = await api.listProjects();

    // 第一次加载才恢复上次项目；之后的 reload 必须尊重用户当前的选择。
    if (!startupProjectRestored) {
      startupProjectRestored = true;
      const saved = readSavedProjectId();
      if (saved !== null) {
        const project = state.projects.find((item) => item.id === saved && !item.isTodo);
        if (project && project.id !== null) {
          state.container = { kind: "project", id: project.id };
          state.projectId = project.id;
          state.scope = { kind: "project" };
        } else {
          state.projectId = null;
          try {
            localStorage.removeItem(PROJECT_STORAGE_KEY);
          } catch {
            // 忽略：无效值不阻塞启动。
          }
        }
      }
    }

    // 取成局部量：容器在 await 之间可能被用户切走，一次重画只认一个容器。
    const container = state.container;
    const projectId = state.projectId;
    if (projectId !== null) {
      state.modules = await api.listModules(projectId);
      state.projectTasks =
        container.kind === "project" && container.id === projectId
          ? []
          : await api.listTasks({ kind: "project", id: projectId });
    } else {
      state.modules = [];
      state.projectTasks = [];
    }
    state.tasks = await api.listTasks(container);
    if (projectId !== null && container.kind === "project" && container.id === projectId) {
      state.projectTasks = state.tasks;
    }
    pruneSubtaskInput();
    render();
  } catch (error) {
    showNotice(describe(error), "error");
  }
}

async function selectProject(project: Project): Promise<void> {
  const isTodo = project.isTodo || project.id === null;
  state.container = isTodo ? { kind: "todo" } : { kind: "project", id: project.id as number };
  // 切到「我的待办」不清空项目上下文：左栏项目名和模块树要继续可见。
  if (!isTodo) state.projectId = project.id;
  // 换容器等于换了一棵树：范围回到"整个项目"，收起状态也没有意义了。
  state.scope = { kind: "project" };
  state.collapsedNodes.clear();
  state.collapsedGroups.clear();
  state.collapsedTasks.clear();
  state.addingSubtaskFor = null;
  subtaskDraft = "";
  persistCurrentProject();
  closeMenu();
  await reload();
}

/** 父任务没了（删除或换容器）就把内联输入收回，别停在一个不存在的父任务上。 */
function pruneSubtaskInput(): void {
  if (state.addingSubtaskFor === null) return;
  const parent = state.tasks.find(
    (task) => task.id === state.addingSubtaskFor && task.parentTaskId === null,
  );
  if (!parent) {
    state.addingSubtaskFor = null;
    subtaskDraft = "";
  }
}

// ---------------------------------------------------------------------------
// 渲染
// ---------------------------------------------------------------------------

function render(): void {
  // 表格用当前容器的任务；左栏树始终展示项目上下文（切到「我的待办」时也保留）。
  const groups = buildGroups(state.tasks, state.modules);
  const sidebarGroups =
    state.projectId !== null ? buildGroups(state.projectTasks, state.modules) : groups;
  renderSidebar(sidebarGroups);
  renderToolbar(groups);
  renderTable(groups);
  // 数据量变化可能让纵向滚动条出现 / 消失，任务列的剩余宽度要跟着重算。
  applyColumnWidths();
}

function renderSidebar(groups: Group[]): void {
  dom.todoSection.replaceChildren();

  const todo = state.projects.find((project) => project.isTodo);
  if (todo) dom.todoSection.append(renderSideItem(todo));

  const project = currentProject();
  const hasProject = state.projectId !== null && project !== undefined && !project.isTodo;
  dom.projectHeading.textContent = hasProject ? project.name : "";
  dom.projectHeading.hidden = !hasProject;

  renderScopeTree(groups);
}

function renderSideItem(project: Project): HTMLButtonElement {
  const button = el("button", "side-item");
  button.type = "button";
  button.classList.toggle("selected", isCurrent(project));
  button.title = project.name;

  button.append(
    el("span", "side-name", project.name),
    el("span", "side-count", String(project.taskCount)),
  );
  button.addEventListener("click", () => {
    void selectProject(project);
  });

  // 右键：项目上没有结构可管时不打开菜单（「我的待办」就属于这种）。
  // 原生右键菜单由 boot() 里的全局监听统一禁用。
  button.addEventListener("contextmenu", (event) => {
    if (project.isTodo || project.id === null) return;
    const projectId = project.id;
    event.preventDefault();
    openMenu({
      items: [
        { label: "新建模块", onSelect: () => void createModuleIn(projectId) },
        {
          label: "重命名项目",
          onSelect: () => void renameNode({ kind: "project", id: projectId, name: project.name }),
        },
        { label: "删除项目", onSelect: () => void deleteProjectFlow(project) },
      ],
      position: { kind: "point", x: event.clientX, y: event.clientY },
    });
  });

  return button;
}

function isCurrent(project: Project): boolean {
  if (project.isTodo) return state.container.kind === "todo";
  return project.id !== null && state.projectId === project.id;
}

function currentProject(): Project | undefined {
  if (state.projectId !== null) {
    const project = state.projects.find((p) => p.id === state.projectId);
    if (project) return project;
  }
  return state.projects.find((p) => p.isTodo);
}

/**
 * 左栏范围树：整个项目 / 模块 / 子模块 / 默认。点哪个就把主视图限定在哪个子树。
 * 「我的待办」没有模块，也就没有这层树。
 */
function renderScopeTree(groups: Group[]): void {
  dom.scopeSection.replaceChildren();

  const visible = state.projectId !== null;
  dom.scopeSection.hidden = !visible;
  if (!visible) return;

  dom.scopeSection.append(
    renderScopeNode({
      scope: { kind: "project" },
      label: "全部",
      count: countGroups(groups),
      indent: 0,
    }),
  );

  // 「默认」排在模块之前，与表格分组顺序一致；空默认分组不占一行。
  const unclassified = groups.find((group) => group.kind === "unclassified");
  if (unclassified) {
    dom.scopeSection.append(
      renderScopeNode({
        scope: { kind: "unclassified" },
        label: unclassified.label,
        count: countGroup(unclassified),
        indent: 0,
      }),
    );
  }

  for (const group of groups) {
    if (group.kind !== "module" || group.id === null) continue;
    const collapsed = state.collapsedNodes.has(group.id);

    dom.scopeSection.append(
      renderScopeNode({
        scope: { kind: "module", id: group.id },
        label: group.label,
        count: countGroup(group),
        indent: 0,
        ...(group.children.length > 0 ? { collapsible: { id: group.id, collapsed } } : {}),
      }),
    );
    if (collapsed) continue;

    for (const child of group.children) {
      if (child.id === null) continue;
      dom.scopeSection.append(
        renderScopeNode({
          scope: { kind: "submodule", id: child.id },
          label: child.label,
          count: countGroup(child),
          indent: 1,
        }),
      );
    }
  }
}

interface ScopeNodeOptions {
  scope: Scope;
  label: string;
  count: { done: number; total: number };
  indent: 0 | 1;
  /** 只有带子模块的模块节点才有折叠箭头 */
  collapsible?: { id: number; collapsed: boolean };
}

function renderScopeNode(options: ScopeNodeOptions): HTMLElement {
  const row = el("div", "scope-row");
  row.classList.toggle("selected", isScopeCurrent(options.scope));
  row.style.paddingLeft = `${8 + options.indent * 16}px`;

  if (options.collapsible) {
    const { id, collapsed } = options.collapsible;
    // 三角由 CSS 的 ::before 画，元素里不要放字符：
    // 字面 ▾ 叠上伪元素就是两个三角（已经错过一次）。
    const caret = el("button", collapsed ? "scope-caret is-collapsed has-icon" : "scope-caret has-icon");
    caret.type = "button";
    caret.title = collapsed ? "展开子模块" : "收起子模块";
    caret.setAttribute("aria-label", caret.title);
    caret.append(icon("chevron"));
    caret.addEventListener("click", () => {
      toggleNode(id);
    });
    row.append(caret);
  } else {
    row.append(el("span", "scope-caret"));
  }

  const name = el("button", "scope-name", options.label);
  name.type = "button";
  name.title = options.label;
  name.addEventListener("click", () => {
    applyScope(options.scope);
  });

  row.append(name, el("span", "scope-count", `${options.count.done}/${options.count.total}`));

  row.addEventListener("contextmenu", (event) => {
    const items = scopeMenuItems(options);
    if (items.length === 0) return;
    event.preventDefault();
    openMenu({ items, position: { kind: "point", x: event.clientX, y: event.clientY } });
  });

  return row;
}

/** 右键某个范围节点时能做什么。结构建在哪一级，菜单就长在哪一级。 */
function scopeMenuItems(options: ScopeNodeOptions): MenuItem[] {
  const { scope, label } = options;

  switch (scope.kind) {
    case "project": {
      const projectId = state.projectId;
      return projectId === null
        ? []
        : [{ label: "新建模块", onSelect: () => void createModuleIn(projectId) }];
    }
    case "module":
      return [
        { label: "新建子模块", onSelect: () => void createSubmoduleIn(scope.id) },
        {
          label: "重命名模块",
          onSelect: () => void renameNode({ kind: "module", id: scope.id, name: label }),
        },
        { label: "删除模块", onSelect: () => void deleteNodeFlow("module", scope.id, label) },
      ];
    case "submodule":
      return [
        {
          label: "重命名子模块",
          onSelect: () => void renameNode({ kind: "submodule", id: scope.id, name: label }),
        },
        { label: "删除子模块", onSelect: () => void deleteNodeFlow("submodule", scope.id, label) },
      ];
    case "unclassified":
      return [];
  }
}

function isScopeCurrent(scope: Scope): boolean {
  if (state.container.kind !== "project") return false;
  if (scope.kind !== state.scope.kind) return false;
  if (scope.kind === "project" || scope.kind === "unclassified") return true;
  return scope.kind === state.scope.kind && "id" in state.scope && scope.id === state.scope.id;
}

function applyScope(scope: Scope): void {
  state.scope = scope;
  // 在「我的待办」里点模块树：切回当前项目，让表格显示对应范围。
  if (state.container.kind === "todo" && state.projectId !== null) {
    state.container = { kind: "project", id: state.projectId };
    state.addingSubtaskFor = null;
    subtaskDraft = "";
    void reload();
    return;
  }
  render();
}

function toggleNode(id: number): void {
  if (state.collapsedNodes.has(id)) state.collapsedNodes.delete(id);
  else state.collapsedNodes.add(id);
  render();
}

function renderToolbar(groups: Group[]): void {
  const inProject = state.container.kind === "project";
  const visible = inProject ? selectScope(groups, state.scope) : groups;
  const { done, total } = countGroups(visible);

  renderFilters();
  dom.statusCount.textContent = total === 0 ? "暂无任务" : `共 ${total} 项 · 已完成 ${done}`;

  // 提示语跟着范围走：新任务会落到哪里，先写清楚，不要让人点完才知道。
  // 「全部」与「默认」的新任务都进默认分组，提示也保持一致。
  const target = state.scope.kind === "project" ? "默认" : scopeLabel(groups, state.scope);
  dom.taskInput.placeholder = inProject
    ? `添加到「${target}」，回车保存（优先级中）`
    : "添加任务，回车保存（优先级中）";
}

function currentFilters(): TaskFilters {
  return { statuses: state.statusFilter, priorities: state.priorityFilter };
}

type FilterKind = "status" | "priority";

function renderFilters(): void {
  dom.filters.replaceChildren();
  dom.filters.append(
    renderFilterGroup(
      "status",
      "状态",
      STATUS_ORDER.map((status) => ({
        value: status,
        label: STATUS_LABELS[status],
        active: state.statusFilter.has(status),
      })),
    ),
    renderFilterGroup(
      "priority",
      "优先级",
      PRIORITY_ORDER.map((priority) => ({
        value: priority,
        label: PRIORITY_LABELS[priority],
        active: state.priorityFilter.has(priority),
        priority,
      })),
    ),
  );
}

interface FilterOption {
  value: string;
  label: string;
  active: boolean;
  priority?: Priority;
}

function renderFilterGroup(kind: FilterKind, label: string, options: FilterOption[]): HTMLElement {
  const group = el("div", "filter-group");
  group.setAttribute("role", "group");
  group.setAttribute("aria-label", `${label}筛选`);
  group.append(el("span", "filter-label", label));

  for (const option of options) {
    const button = el("button", "filter-chip", option.label);
    button.type = "button";
    button.dataset.filterKind = kind;
    button.dataset.filterValue = option.value;
    if (option.priority) button.dataset.priority = option.priority;
    button.setAttribute("aria-pressed", String(option.active));
    button.addEventListener("click", () => {
      toggleFilter(kind, option.value);
    });
    group.append(button);
  }

  return group;
}

function toggleFilter(kind: FilterKind, value: string): void {
  if (kind === "status") {
    const status = value as Status;
    if (state.statusFilter.has(status)) state.statusFilter.delete(status);
    else state.statusFilter.add(status);
  } else {
    const priority = value as Priority;
    if (state.priorityFilter.has(priority)) state.priorityFilter.delete(priority);
    else state.priorityFilter.add(priority);
  }

  render();
  // 全量重画会替换按钮；焦点送回刚切换的标签，键盘连续操作不中断。
  dom.filters
    .querySelector<HTMLButtonElement>(
      `button[data-filter-kind="${kind}"][data-filter-value="${value}"]`,
    )
    ?.focus();
}

/** 子任务按父任务归桶；桶内与组内父任务同一套排序（优先级 → 添加顺序）。 */
function groupChildren(tasks: readonly Task[]): Map<number, Task[]> {
  const map = new Map<number, Task[]>();
  for (const task of tasks) {
    if (task.parentTaskId === null) continue;
    const bucket = map.get(task.parentTaskId);
    if (bucket) bucket.push(task);
    else map.set(task.parentTaskId, [task]);
  }
  for (const [parentId, children] of map) map.set(parentId, sortTasks(children));
  return map;
}

function renderTable(groups: Group[]): void {
  const childrenByParent = groupChildren(state.tasks);
  const filters = currentFilters();
  dom.body.replaceChildren();

  // 「我的待办」没有模块，也就没有分组行：表格直接平铺。
  if (state.container.kind === "todo") {
    const parents = sortTasks(state.tasks.filter((task) => task.parentTaskId === null));
    for (const task of parents) {
      renderTaskTree(task, childrenByParent.get(task.id) ?? [], dom.body, filters);
    }
    const hasMatch = parents.some((task) =>
      taskTreeMatchesFilters(task, childrenByParent.get(task.id) ?? [], filters),
    );
    setEmptyState(
      !hasMatch,
      parents.length === 0
        ? "「我的待办」里还没有任务。在下面输入一行，回车保存。"
        : "当前筛选下没有任务。调整上方的状态或优先级标签试试。",
    );
    focusSubtaskInput();
    return;
  }

  const visible = selectScope(groups, state.scope);
  for (const group of visible) renderGroup(group, dom.body, childrenByParent, filters);

  const { total } = countGroups(visible);
  const where = state.scope.kind === "project" ? "这个项目" : `「${scopeLabel(groups, state.scope)}」`;
  const hasMatch = visible.some((group) => groupHasMatchingTaskTree(group, childrenByParent, filters));
  setEmptyState(
    !hasMatch,
    total === 0
      ? `${where}还没有任务。在下面输入一行，回车保存。`
      : "当前筛选下没有任务。调整上方的状态或优先级标签试试。",
  );
  focusSubtaskInput();
}

function setEmptyState(empty: boolean, message: string): void {
  dom.empty.hidden = !empty;
  dom.empty.textContent = empty ? message : "";
}

function taskTreeMatchesFilters(
  task: Task,
  children: readonly Task[],
  filters: TaskFilters,
): boolean {
  return taskMatchesFilters(task, filters) || children.some((child) => taskMatchesFilters(child, filters));
}

function groupHasMatchingTaskTree(
  group: Group,
  childrenByParent: Map<number, Task[]>,
  filters: TaskFilters,
): boolean {
  return (
    group.tasks.some((task) =>
      taskTreeMatchesFilters(task, childrenByParent.get(task.id) ?? [], filters),
    ) ||
    group.children.some((child) => groupHasMatchingTaskTree(child, childrenByParent, filters))
  );
}

function groupKey(group: Group): string {
  return `${group.kind}:${group.id ?? "-"}`;
}

/** 深度优先：分组行 → 组内任务（含子任务） → 子分组（缩进一级）。 */
function renderGroup(
  group: Group,
  host: HTMLElement,
  childrenByParent: Map<number, Task[]>,
  filters: TaskFilters,
): void {
  const key = groupKey(group);
  const collapsed = state.collapsedGroups.has(key);
  const { done, total } = countGroup(group);

  const row = el("tr", group.depth === 1 ? "row-subgroup" : "row-group");
  const cell = el("td");
  cell.colSpan = 7;

  const label = el("span", "group-label");
  label.style.paddingLeft = `${group.depth * 20}px`;

  const caret = el("button", collapsed ? "group-caret is-collapsed has-icon" : "group-caret has-icon");
  caret.type = "button";
  caret.title = collapsed ? "展开" : "收起";
  caret.setAttribute("aria-label", caret.title);
  caret.append(icon("chevron"));
  caret.addEventListener("click", () => {
    if (collapsed) state.collapsedGroups.delete(key);
    else state.collapsedGroups.add(key);
    render();
  });

  label.append(caret, el("span", undefined, group.label));
  if (group.children.length > 0) {
    label.append(el("span", "group-meta", `${group.children.length} 个子模块`));
  }
  label.append(el("span", "group-count", `${done}/${total}`));

  cell.append(label);
  row.append(cell);
  host.append(row);

  if (collapsed) return;

  for (const task of group.tasks) {
    renderTaskTree(task, childrenByParent.get(task.id) ?? [], host, filters);
  }
  for (const child of group.children) renderGroup(child, host, childrenByParent, filters);
}

interface TaskRowOptions {
  isChild: boolean;
  /** 当前筛选下会显示的子任务数，决定是否画折叠箭头。 */
  childCount: number;
  /** 子任务完成计数始终按全部子任务算，不受筛选影响。 */
  childDone: number;
  childTotal: number;
  isContext?: boolean;
}

/**
 * 父任务 + 默认展开的子任务 + 内联新增行。
 *
 * 父任务不命中但子任务命中时，父行作为灰显上下文保留；两者都不命中才整棵隐藏。
 */
function renderTaskTree(
  task: Task,
  children: readonly Task[],
  host: HTMLElement,
  filters: TaskFilters,
): void {
  const matchingChildren = filterTasks(children, filters);
  const parentMatches = taskMatchesFilters(task, filters);
  if (!parentMatches && matchingChildren.length === 0) return;

  host.append(
    renderTaskRow(task, {
      isChild: false,
      childCount: matchingChildren.length,
      childDone: children.filter((child) => child.status === "done").length,
      childTotal: children.length,
      isContext: !parentMatches,
    }),
  );

  if (!state.collapsedTasks.has(task.id)) {
    for (const child of matchingChildren) {
      host.append(
        renderTaskRow(child, { isChild: true, childCount: 0, childDone: 0, childTotal: 0 }),
      );
    }
  }

  if (state.addingSubtaskFor === task.id) host.append(renderSubtaskAddRow(task));
}

function renderTaskRow(task: Task, options: TaskRowOptions): HTMLTableRowElement {
  const row = el("tr", "row-task");
  row.dataset.taskId = String(task.id);
  if (options.isChild) row.classList.add("row-subtask");
  if (options.isContext) {
    row.classList.add("is-context");
    row.title = "筛选上下文：这个父任务本身不匹配当前筛选";
  }
  if (task.status === "done") row.classList.add("is-done");

  row.append(
    renderStatusCell(task),
    renderTitleCell(task, options),
    renderPriorityCell(task),
    renderTimeCell(task, "created"),
    renderTimeCell(task, "start"),
    renderTimeCell(task, "end"),
    renderNoteCell(task),
  );

  // 行内正在编辑时不抢右键：输入框里的右键仍交给浏览器默认行为处理。
  row.addEventListener("contextmenu", (event) => {
    if (
      event.target instanceof HTMLInputElement ||
      event.target instanceof HTMLTextAreaElement
    ) {
      return;
    }
    event.preventDefault();
    openTaskMenu(task, event);
  });

  return row;
}

/** 任务右键菜单：移动 / 升降级（顶层与子任务各一项）与删除。 */
function openTaskMenu(task: Task, event: MouseEvent): void {
  const items: MenuItem[] = [];
  const position: MenuPosition = { kind: "point", x: event.clientX, y: event.clientY };
  if (task.parentTaskId === null) {
    items.push({ label: "移动到…", onSelect: () => void openMoveMenu(task, position) });
    items.push({ label: "降为子任务…", onSelect: () => openDemoteMenu(task, position) });
  } else {
    items.push({ label: "升为父任务", onSelect: () => void promoteTask(task) });
  }
  items.push({ label: "删除任务", onSelect: () => void deleteTaskFlow(task) });
  openMenu({ items, position });
}

/** 目标父任务按当前容器的分组展示；子任务不能当父任务。 */
function demoteTargetItem(task: Task, target: Task): MenuItem {
  return { label: target.title, onSelect: () => void demoteTaskTo(task, target) };
}

function demoteGroupItem(task: Task, group: Group): MenuItem | null {
  const tasks = sortTasks(group.tasks.filter((candidate) => candidate.id !== task.id));
  const children = group.children
    .map((child) => demoteGroupItem(task, child))
    .filter((child): child is MenuItem => child !== null);

  if (tasks.length === 0 && children.length === 0) return null;
  return {
    label: group.label,
    children: [...tasks.map((target) => demoteTargetItem(task, target)), ...children],
  };
}

/** 「降为子任务…」的第二级菜单：目标只能是当前容器里的顶层任务。 */
function openDemoteMenu(task: Task, position: MenuPosition): void {
  const children = state.tasks.filter((candidate) => candidate.parentTaskId === task.id);
  if (children.length > 0) {
    showNotice(
      `「${task.title}」还有 ${children.length} 个子任务，不能直接降级；请先删除或升为父任务`,
    );
    return;
  }

  const targets = state.tasks.filter(
    (candidate) => candidate.parentTaskId === null && candidate.id !== task.id,
  );
  if (targets.length === 0) {
    showNotice("没有可挂靠的父任务");
    return;
  }

  const items =
    state.container.kind === "todo"
      ? sortTasks(targets).map((target) => demoteTargetItem(task, target))
      : buildGroups(state.tasks, state.modules)
          .map((group) => demoteGroupItem(task, group))
          .filter((item): item is MenuItem => item !== null);

  openMenu({ items, position, label: "降为子任务" });
}

/** 降级成功才重画；目标折叠着就先展开，让新子任务能被看到。 */
async function demoteTaskTo(task: Task, target: Task): Promise<void> {
  const clamped = PRIORITY_ORDER.indexOf(task.priority) < PRIORITY_ORDER.indexOf(target.priority);
  try {
    const outcome = await api.reparentTask(task.id, target.id);
    state.collapsedTasks.delete(target.id);
    await reload();
    flashTaskTree(target.id);
    showNotice(
      clamped
        ? `已将「${task.title}」降为「${target.title}」的子任务，优先级调整为${PRIORITY_LABELS[target.priority]}`
        : `已将「${task.title}」降为「${target.title}」的子任务`,
    );
    for (const notice of outcome.notices) showNotice(notice);
  } catch (error) {
    showNotice(describe(error), "error");
  }
}

/** 升级成功才重画；原父任务若被聚合改动，提示由后端带回。 */
async function promoteTask(task: Task): Promise<void> {
  try {
    const outcome = await api.reparentTask(task.id, null);
    await reload();
    flashTaskTree(task.id);
    showNotice(`已将「${task.title}」升为父任务`);
    for (const notice of outcome.notices) showNotice(notice);
  } catch (error) {
    showNotice(describe(error), "error");
  }
}

/**
 * 创建 / 开始 / 结束各一列，只读。创建时间在新建任务时写入；开始与结束
 * 随状态变化自动维护。表格里没有录入入口。
 */
function renderTimeCell(task: Task, field: TimeField): HTMLTableCellElement {
  const cell = el("td", "cell-time");
  const value =
    field === "created" ? task.createdAt : field === "start" ? task.startAt : task.endAt;
  const label = `${TIME_LABELS[field]}时间`;
  const span = el("span", value === null ? "time is-empty" : "time", formatSchedule(value));
  span.title = `${label}：${value === null ? "未记录" : formatSchedule(value)}（${
    TIME_HINTS[field]
  }）`;
  span.setAttribute("aria-label", span.title);
  cell.append(span);
  return cell;
}

function renderStatusCell(task: Task): HTMLTableCellElement {
  const cell = el("td", "cell-status");
  const button = el("button", `status status-${task.status}`);
  button.type = "button";
  button.title = `${STATUS_LABELS[task.status]} → 点一下改为${STATUS_LABELS[NEXT_STATUS[task.status]]}`;
  button.setAttribute("aria-label", `状态：${STATUS_LABELS[task.status]}`);
  button.append(
    icon(
      task.status === "done"
        ? "status-done"
        : task.status === "doing"
          ? "status-doing"
          : "status-todo",
    ),
  );
  button.addEventListener("click", () => {
    void cycleStatus(task);
  });
  cell.append(button);
  return cell;
}

function renderPriorityCell(task: Task): HTMLTableCellElement {
  const cell = el("td", "cell-priority");
  const button = el("button", `priority priority-${task.priority}`, PRIORITY_LABELS[task.priority]);
  button.type = "button";
  button.title =
    task.parentTaskId === null
      ? `优先级${PRIORITY_LABELS[task.priority]}（点击选择）`
      : `优先级${PRIORITY_LABELS[task.priority]}（点击选择；不能高于父任务）`;
  button.setAttribute("aria-haspopup", "menu");
  button.setAttribute("aria-expanded", "false");
  button.addEventListener("click", () => {
    openPriorityMenu(button, task);
  });
  cell.append(button);
  return cell;
}

function renderTitleCell(task: Task, options: TaskRowOptions): HTMLTableCellElement {
  const cell = el("td", "cell-title");
  const title = el("span", "task-title", task.title);
  title.title = "双击改名";
  title.addEventListener("dblclick", () => beginRename(title, task));

  // 子任务到头了：没有箭头、没有「＋」，只靠缩进表达层级。
  if (options.isChild) {
    cell.append(title);
    return cell;
  }

  const wrap = el("div", "task-title-row");
  const collapsed = state.collapsedTasks.has(task.id);

  if (options.childCount > 0) {
    const caret = el(
      "button",
      collapsed ? "group-caret task-caret is-collapsed has-icon" : "group-caret task-caret has-icon",
    );
    caret.type = "button";
    caret.title = collapsed ? "展开子任务" : "收起子任务";
    caret.setAttribute("aria-label", caret.title);
    caret.setAttribute("aria-expanded", String(!collapsed));
    caret.append(icon("chevron"));
    caret.addEventListener("click", () => toggleTaskCollapsed(task.id));
    wrap.append(caret);
  } else {
    // 没有子任务也留出箭头宽度，标题不会因为有了子任务而左右跳。
    wrap.append(el("span", "task-caret task-caret-empty"));
  }

  wrap.append(title);

  if (options.childTotal > 0) {
    const count = el("span", "task-count", `${options.childDone}/${options.childTotal}`);
    count.title = `子任务已完成 ${options.childDone} 个，共 ${options.childTotal} 个`;
    wrap.append(count);
  }

  const plus = el("button", "subtask-add");
  plus.type = "button";
  plus.title = "添加子任务";
  plus.setAttribute("aria-label", `给「${task.title}」添加子任务`);
  plus.append(icon("plus"));
  plus.addEventListener("click", () => openSubtaskInput(task.id));
  wrap.append(plus);

  cell.append(wrap);
  return cell;
}

function toggleTaskCollapsed(id: number): void {
  if (state.collapsedTasks.has(id)) state.collapsedTasks.delete(id);
  else state.collapsedTasks.add(id);
  render();
}

function openSubtaskInput(parentId: number): void {
  // 无论是否已经在输入，都作废上一轮输入框的失焦回调。
  subtaskInputSession += 1;
  state.collapsedTasks.delete(parentId); // 输入行跟在子任务后面，必须先展开

  if (state.addingSubtaskFor === parentId) {
    focusSubtaskInput();
    return;
  }

  state.addingSubtaskFor = parentId;
  subtaskDraft = "";
  render();
}

/** 父任务行下的内联子任务输入：回车连续添加，Esc 取消，失焦保存非空内容。 */
function renderSubtaskAddRow(parent: Task): HTMLTableRowElement {
  const session = subtaskInputSession;
  const row = el("tr", "row-subtask-add");
  row.append(el("td"));
  const cell = el("td", "cell-title");

  const input = el("input", "subtask-input");
  input.value = subtaskDraft;
  input.placeholder = "添加子任务，回车保存";
  input.autocomplete = "off";
  input.spellcheck = false;

  let settled = false;
  const finish = (commit: boolean, keepOpen: boolean): void => {
    if (settled) return;
    settled = true;

    const title = input.value.trim();
    state.addingSubtaskFor = keepOpen ? parent.id : null;
    subtaskDraft = "";

    if (!commit || title === "") {
      render();
      return;
    }

    // 先重绘清空输入框，再发命令；失败会把草稿放回来。
    render();
    void submitSubtask(parent, title);
  };

  input.addEventListener("input", () => {
    subtaskDraft = input.value;
  });
  input.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      finish(true, true);
    } else if (event.key === "Escape") {
      event.preventDefault();
      finish(false, false);
    }
  });
  // 失焦保存非空内容，与就地改名同一理由。延后一拍：点另一个「＋」时
  // blur 先于 click，同步重绘会把按钮换掉、吞掉紧接着的 click。
  input.addEventListener("blur", () => {
    window.setTimeout(() => {
      // `isConnected` 用来区分“用户点了别处”和“重绘把这个输入框换掉了”：
      // 后者也会触发 blur，但输入框已经不在文档里，不能当成提交。
      if (settled || !input.isConnected) return;
      if (session !== subtaskInputSession) return;
      if (state.addingSubtaskFor !== parent.id) return;
      finish(true, false);
    }, 0);
  });

  cell.append(input);
  row.append(cell);
  for (let index = 0; index < 5; index += 1) row.append(el("td"));
  return row;
}

function focusSubtaskInput(): void {
  if (state.addingSubtaskFor === null) return;
  const input = dom.body.querySelector<HTMLInputElement>(".subtask-input");
  if (!input) return;
  input.focus();
  input.setSelectionRange(input.value.length, input.value.length);
}

async function submitSubtask(parent: Task, title: string): Promise<void> {
  try {
    const created = await api.createTask(
      state.container,
      { moduleId: null, submoduleId: null },
      title,
      parent.id,
    );
    await reload();
    for (const notice of created.notices) showNotice(notice);
  } catch (error) {
    // 失败把草稿放回去，别因为一次后端拒绝丢掉刚打的字。
    state.addingSubtaskFor = parent.id;
    subtaskDraft = title;
    render();
    showNotice(describe(error), "error");
  }
}

function renderNoteCell(task: Task): HTMLTableCellElement {
  const cell = el("td", "cell-note");
  // 摘要：换行与连续空白折成单个空格，DOM 里永远是单行。
  const summary = task.note.trim().replace(/\s+/g, " ");
  const button = el("button", summary ? "note" : "note is-empty", summary || "—");
  button.type = "button";
  button.title = "编辑备注";
  button.setAttribute("aria-label", `备注：${summary || "空"}`);
  button.addEventListener("click", () => void editNote(task));
  cell.append(button);
  return cell;
}

// ---------------------------------------------------------------------------
// 写操作（格内编辑）
// ---------------------------------------------------------------------------

/** 备注：弹窗里原样编辑，空串表示清空；没改就不发命令。 */
async function editNote(task: Task): Promise<void> {
  const note = await askNote({
    title: "备注",
    label: `「${task.title}」`,
    value: task.note,
    confirmText: "保存",
  });
  if (note === null || note === task.note) return;

  try {
    await api.setNote(task.id, note);
    await reload();
  } catch (error) {
    showNotice(describe(error), "error");
  }
}

function isSameTarget(task: Task, target: MoveTarget): boolean {
  const { container } = target;
  const sameContainer =
    container.kind === "todo" ? task.projectId === null : task.projectId === container.id;
  return (
    sameContainer &&
    task.moduleId === target.moduleId &&
    task.submoduleId === target.submoduleId
  );
}

/** 当前目标不能再选一次，直接不出现在菜单里。 */
function moveTargetItem(task: Task, label: string, target: MoveTarget): MenuItem | null {
  return isSameTarget(task, target)
    ? null
    : { label, onSelect: () => void moveTaskTo(task, target) };
}

/**
 * 移动目标按「项目 → 模块 → 子模块」折叠成树：默认只露出我的待办和项目，
 * 展开项目才看到默认和模块，模块再展开才看到子模块。
 * 菜单里的短标签靠层级表达归属，不再重复写完整的 `项目 › 模块 › 子模块`。
 */
async function moveMenuItems(task: Task): Promise<MenuItem[]> {
  const items: MenuItem[] = [];
  const todo = moveTargetItem(task, "我的待办", {
    container: { kind: "todo" },
    moduleId: null,
    submoduleId: null,
  });
  if (todo) items.push(todo);

  const projects = state.projects.filter(
    (project): project is Project & { id: number } => project.id !== null,
  );
  const trees = await Promise.all(projects.map((project) => api.listModules(project.id)));

  projects.forEach((project, index) => {
    const container: Container = { kind: "project", id: project.id };
    const children: MenuItem[] = [];

    const unclassified = moveTargetItem(task, "默认", {
      container,
      moduleId: null,
      submoduleId: null,
    });
    if (unclassified) children.push(unclassified);

    for (const module of trees[index]) {
      const moduleTarget: MoveTarget = { container, moduleId: module.id, submoduleId: null };
      const submoduleItems = module.submodules.flatMap((submodule) => {
        const item = moveTargetItem(task, submodule.name, {
          container,
          moduleId: module.id,
          submoduleId: submodule.id,
        });
        return item === null ? [] : [item];
      });

      const wholeModule = moveTargetItem(task, "整个模块", moduleTarget);
      if (submoduleItems.length === 0) {
        // 没有子模块可展开：模块直接当叶子项，省掉一层点击。
        if (wholeModule) children.push({ ...wholeModule, label: module.name });
        continue;
      }
      children.push({
        label: module.name,
        children: wholeModule ? [wholeModule, ...submoduleItems] : submoduleItems,
      });
    }

    if (children.length > 0) items.push({ label: project.name, children });
  });

  return items;
}

/**
 * 「移动到…」的第二级菜单：仍用同一个 `openMenu`，在右键位置重新弹出。
 * 项目 → 模块 → 子模块 由菜单组件弹成侧向子面板，父面板不重排。
 * 当前位置不出现在列表里。
 */
async function openMoveMenu(task: Task, position: MenuPosition): Promise<void> {
  try {
    const items = await moveMenuItems(task);
    if (items.length === 0) {
      showNotice("没有可移动到的位置");
      return;
    }

    openMenu({ items, position, label: "移动到" });
  } catch (error) {
    showNotice(describe(error), "error");
  }
}

/** 移动成功才重画并提示；失败保持原样，别把行凭空弄丢。 */
async function moveTaskTo(task: Task, target: MoveTarget): Promise<void> {
  try {
    await api.moveTask(task.id, target);
    await reload();
    flashTaskTree(task.id);
    showNotice(`已移动「${task.title}」`);
  } catch (error) {
    showNotice(describe(error), "error");
  }
}

/** 状态格循环：plan 先问"要不要确认"，needs_confirm 为真才弹窗，再 apply。 */
async function cycleStatus(task: Task): Promise<void> {
  const next = NEXT_STATUS[task.status];
  try {
    const plan = await api.planSetStatus(task.id, next);
    let cascade = false;

    if (plan.needsConfirm) {
      const confirmed = await askConfirm({
        title: `标记为${STATUS_LABELS[next]}？`,
        body: plan.reason,
        confirmText: "确定",
      });
      if (!confirmed) return;
      cascade = true;
    }

    const outcome = await api.applySetStatus(task.id, next, cascade);
    await reload();
    // 「已完成」未勾选时，任务完成会立刻离开视图；先说明它去哪了，别让操作看起来没生效。
    const stillVisible = document.querySelector(`tr[data-task-id="${task.id}"]`) !== null;
    if (next === "done" && !stillVisible) {
      showNotice("任务已完成，已从当前视图隐藏");
    }
    for (const notice of outcome.notices) showNotice(notice);
  } catch (error) {
    showNotice(describe(error), "error");
  }
}

/** 优先级浮窗。改完行会按组内排序跳走，给新位置一次短暂高亮。 */
function openPriorityMenu(anchor: HTMLButtonElement, task: Task): void {
  // 子任务的可选档位以父任务为上限：更高的选项根本不出现。
  const parent =
    task.parentTaskId === null
      ? null
      : (state.tasks.find((candidate) => candidate.id === task.parentTaskId) ?? null);
  const choices =
    parent === null
      ? PRIORITY_ORDER
      : PRIORITY_ORDER.slice(PRIORITY_ORDER.indexOf(parent.priority));

  openMenu({
    items: choices.map((priority) => ({
      label: PRIORITY_LABELS[priority],
      checked: priority === task.priority,
      onSelect: () => {
        if (priority !== task.priority) void applyPriority(task, priority);
      },
    })),
    position: { kind: "anchor", element: anchor },
    label: "优先级",
    restoreFocus: anchor,
  });
}

async function applyPriority(task: Task, priority: Priority): Promise<void> {
  // 降到比某些子任务还低时，后端会把它们一并降档；先数一下，回来给条提示。
  const cascaded =
    task.parentTaskId === null
      ? state.tasks.filter(
          (child) =>
            child.parentTaskId === task.id &&
            PRIORITY_ORDER.indexOf(child.priority) < PRIORITY_ORDER.indexOf(priority),
        ).length
      : 0;

  try {
    await api.setPriority(task.id, priority);
    await reload();
    if (cascaded > 0) showNotice(`已同步下调 ${cascaded} 个子任务的优先级`);
    flashTaskTree(task.id);
  } catch (error) {
    showNotice(describe(error), "error");
  }
}

/** 就地改名：只替换标题本身，父任务的箭头、计数和「＋」留在原位。 */
function beginRename(title: HTMLSpanElement, task: Task): void {
  const cell = title.closest<HTMLTableCellElement>("td");
  if (!cell || cell.querySelector(".title-input")) return;

  const input = el("input", "title-input");
  input.value = task.title;
  input.spellcheck = false;
  // 父任务的标题在 flex 行里：输入框从标题原位铺到单元格右侧，计数和「＋」先让位。
  const row = title.parentElement;
  if (row?.classList.contains("task-title-row")) {
    row.classList.add("is-renaming");
  }
  title.replaceWith(input);
  input.focus();
  input.select();

  // 回车与失焦可能先后发生，只认第一个结果。
  let settled = false;
  const finish = (commit: boolean): void => {
    if (settled) return;
    settled = true;

    const next = input.value.trim();
    // 空值或没改：还原原文，不入库（后端也会拒，但没必要发这一趟）。
    if (!commit || next === "" || next === task.title) {
      render();
      return;
    }
    void commitRename(task, next);
  };

  input.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      finish(true);
    } else if (event.key === "Escape") {
      event.preventDefault();
      finish(false);
    }
  });

  // 点别处也算改完：在表格里换焦点是最常见的"我好了"的动作，
  // 这里丢掉刚打的字比多存一次更糟。Esc 仍是唯一的"不要了"。
  input.addEventListener("blur", () => finish(true));
}

async function commitRename(task: Task, title: string): Promise<void> {
  try {
    await api.renameTask(task.id, title);
    await reload();
  } catch (error) {
    showNotice(describe(error), "error");
    render(); // 失败也要把输入框收回去，不能停在一半的状态
  }
}

/** 改完优先级 / 移动完成后，让跳走的父任务和跟着走的子任务行一起闪一下。 */
function flashTaskTree(taskId: number): void {
  const ids = [
    taskId,
    ...state.tasks.filter((task) => task.parentTaskId === taskId).map((task) => task.id),
  ];
  for (const id of ids) {
    dom.body
      .querySelector<HTMLTableRowElement>(`tr[data-task-id="${id}"]`)
      ?.classList.add("just-moved");
  }
}

// ---------------------------------------------------------------------------
// 确认框 / 输入框（同一个 modal）
// ---------------------------------------------------------------------------

/** 每次弹窗前先把上一回的形态抹干净：确认框与输入框共用这一个 modal。 */
function resetModal(): void {
  dom.modalBody.hidden = false;
  dom.modalBody.textContent = "";
  dom.modalField.hidden = true;
  dom.modalInput.hidden = false;
  dom.modalInput.value = "";
  dom.modalTextarea.hidden = true;
  dom.modalTextarea.value = "";
  dom.modalCard.classList.remove("is-wide");
}

/**
 * 当前打开的弹窗的“取消”入口。
 *
 * 同一个 modal 只允许有一个主人：新弹窗会先把旧弹窗按取消收场，
 * 否则两个 `await` 会同时悬着、抢同一批 DOM 监听，一次回车跑两个命令。
 */
let dismissActiveDialog: (() => void) | null = null;

function beginDialog(): void {
  dismissActiveDialog?.();
}

function endDialog(): void {
  dismissActiveDialog = null;
}

interface ConfirmOptions {
  title: string;
  body: string;
  confirmText: string;
}

/** 「先告知影响、再确认」的通用形状。返回 Promise，调用处直接 await。 */
function askConfirm(options: ConfirmOptions): Promise<boolean> {
  return new Promise((resolve) => {
    beginDialog();
    resetModal();
    dom.modalTitle.textContent = options.title;
    dom.modalBody.textContent = options.body;
    dom.modalOk.textContent = options.confirmText;
    dom.modal.hidden = false;
    dom.modalOk.focus();

    const close = (result: boolean): void => {
      endDialog();
      dom.modal.hidden = true;
      dom.modalOk.removeEventListener("click", onOk);
      dom.modalCancel.removeEventListener("click", onCancel);
      dom.modal.removeEventListener("click", onBackdrop);
      document.removeEventListener("keydown", onKey);
      resolve(result);
    };
    const onOk = (): void => close(true);
    const onCancel = (): void => close(false);
    const onBackdrop = (event: MouseEvent): void => {
      if (event.target === dom.modal) close(false);
    };
    const onKey = (event: KeyboardEvent): void => {
      if (event.key === "Escape") close(false);
    };

    dom.modalOk.addEventListener("click", onOk);
    dom.modalCancel.addEventListener("click", onCancel);
    dom.modal.addEventListener("click", onBackdrop);
    document.addEventListener("keydown", onKey);

    dismissActiveDialog = onCancel;
  });
}

interface TextOptions {
  title: string;
  /** 输入框上方的小字，通常写"旧名字 → 新名字" */
  label: string;
  value: string;
  confirmText: string;
}

/** 单个文本输入的弹窗。取消返回 `null`，确认返回 trim 过的非空字符串。 */
function askText(options: TextOptions): Promise<string | null> {
  return new Promise((resolve) => {
    beginDialog();
    resetModal();
    dom.modalTitle.textContent = options.title;
    dom.modalBody.hidden = true;
    dom.modalField.hidden = false;
    dom.modalLabel.textContent = options.label;
    dom.modalInput.value = options.value;
    dom.modalOk.textContent = options.confirmText;
    dom.modal.hidden = false;
    dom.modalInput.focus();
    dom.modalInput.select();

    const close = (result: string | null): void => {
      endDialog();
      dom.modal.hidden = true;
      dom.modalOk.removeEventListener("click", onSubmit);
      dom.modalCancel.removeEventListener("click", onCancel);
      dom.modal.removeEventListener("click", onBackdrop);
      dom.modalInput.removeEventListener("keydown", onInputKey);
      document.removeEventListener("keydown", onKey);
      resolve(result);
    };

    // 空白名不关窗也不出库：关掉等于让用户以为改成了空名字
    const submit = (): void => {
      const value = dom.modalInput.value.trim();
      if (value === "") {
        dom.modalInput.focus();
        return;
      }
      close(value);
    };

    const onSubmit = (): void => submit();
    const onCancel = (): void => close(null);
    const onBackdrop = (event: MouseEvent): void => {
      if (event.target === dom.modal) close(null);
    };
    const onKey = (event: KeyboardEvent): void => {
      if (event.key === "Escape") close(null);
    };
    const onInputKey = (event: KeyboardEvent): void => {
      if (event.key === "Enter") {
        event.preventDefault();
        submit();
      }
    };

    dom.modalOk.addEventListener("click", onSubmit);
    dom.modalCancel.addEventListener("click", onCancel);
    dom.modal.addEventListener("click", onBackdrop);
    dom.modalInput.addEventListener("keydown", onInputKey);
    document.addEventListener("keydown", onKey);

    dismissActiveDialog = onCancel;
  });
}

/**
 * 多行备注弹窗。与 `askText` 的关键差异：
 * - Enter 是换行，不是提交；确认只能点按钮。
 * - 返回值**不 trim**，空串表示清空备注。
 */
function askNote(options: TextOptions): Promise<string | null> {
  return new Promise((resolve) => {
    beginDialog();
    resetModal();
    dom.modalTitle.textContent = options.title;
    dom.modalBody.hidden = true;
    dom.modalField.hidden = false;
    dom.modalLabel.textContent = options.label;
    dom.modalInput.hidden = true;
    dom.modalTextarea.hidden = false;
    dom.modalCard.classList.add("is-wide");
    dom.modalTextarea.value = options.value;
    dom.modalOk.textContent = options.confirmText;
    dom.modal.hidden = false;
    // 光标落在末尾：打开就是为了接着写。
    dom.modalTextarea.focus();
    dom.modalTextarea.setSelectionRange(options.value.length, options.value.length);

    const close = (result: string | null): void => {
      endDialog();
      dom.modal.hidden = true;
      dom.modalCard.classList.remove("is-wide");
      dom.modalOk.removeEventListener("click", onSubmit);
      dom.modalCancel.removeEventListener("click", onCancel);
      dom.modal.removeEventListener("click", onBackdrop);
      document.removeEventListener("keydown", onKey);
      resolve(result);
    };

    const onSubmit = (): void => close(dom.modalTextarea.value);
    const onCancel = (): void => close(null);
    const onBackdrop = (event: MouseEvent): void => {
      if (event.target === dom.modal) close(null);
    };
    const onKey = (event: KeyboardEvent): void => {
      if (event.key === "Escape") close(null);
    };

    dom.modalOk.addEventListener("click", onSubmit);
    dom.modalCancel.addEventListener("click", onCancel);
    dom.modal.addEventListener("click", onBackdrop);
    document.addEventListener("keydown", onKey);

    dismissActiveDialog = onCancel;
  });
}

// ---------------------------------------------------------------------------
// 事件
// ---------------------------------------------------------------------------

dom.taskForm.addEventListener("submit", (event) => {
  event.preventDefault();
  void createTask();
});

async function createProjectFlow(): Promise<void> {
  const name = await askText({
    title: "新建项目",
    label: "项目名",
    value: "",
    confirmText: "创建",
  });
  if (name === null) return;

  try {
    const created = await api.createProject(name);
    await selectProject(created);
  } catch (error) {
    showNotice(describe(error), "error");
  }
}

async function createModuleIn(projectId: number): Promise<void> {
  const name = await askText({
    title: "新建模块",
    label: "模块名",
    value: "",
    confirmText: "创建",
  });
  if (name === null) return;

  try {
    await api.createModule(projectId, name);
    // 在别的项目里建的：把视图切过去，否则看不到自己刚建的东西
    const project = state.projects.find((p) => p.id === projectId);
    if (project && !isCurrent(project)) {
      await selectProject(project);
      return;
    }
    await reload();
  } catch (error) {
    showNotice(describe(error), "error");
  }
}

async function createSubmoduleIn(moduleId: number): Promise<void> {
  const name = await askText({
    title: "新建子模块",
    label: "子模块名",
    value: "",
    confirmText: "创建",
  });
  if (name === null) return;

  try {
    await api.createSubmodule(moduleId, name);
    // 刚加完就看不见它，等于没加：顺手把这层展开
    state.collapsedNodes.delete(moduleId);
    await reload();
  } catch (error) {
    showNotice(describe(error), "error");
  }
}

interface RenameTarget {
  kind: "project" | "module" | "submodule";
  id: number;
  name: string;
}

async function renameNode(target: RenameTarget): Promise<void> {
  const name = await askText({
    title: "重命名",
    label: `${target.name} → 新名字`,
    value: target.name,
    confirmText: "保存",
  });
  if (name === null || name === target.name) return;

  try {
    if (target.kind === "project") await api.renameProject(target.id, name);
    else if (target.kind === "module") await api.renameModule(target.id, name);
    else await api.renameSubmodule(target.id, name);
    await reload();
  } catch (error) {
    showNotice(describe(error), "error");
  }
}

/** 删除任务：先问后端会连带删掉几个子任务，再确认。 */
async function deleteTaskFlow(task: Task): Promise<void> {
  try {
    const plan = await api.planDeleteTask(task.id);
    const extra = plan.taskCount - 1;
    const body =
      extra > 0
        ? `这个任务连同 ${extra} 个子任务会被删除，无法撤销。`
        : "这个任务会被删除，无法撤销。";

    const confirmed = await askConfirm({ title: "删除任务？", body, confirmText: "删除" });
    if (!confirmed) return;

    const outcome = await api.deleteTask(task.id);
    state.collapsedTasks.delete(task.id);
    if (state.addingSubtaskFor === task.id) {
      state.addingSubtaskFor = null;
      subtaskDraft = "";
    }
    await reload();
    showNotice("已删除任务");
    for (const notice of outcome.notices) showNotice(notice);
  } catch (error) {
    showNotice(describe(error), "error");
  }
}

/** 删除模块 / 子模块：组内任务一并删除，条数含子模块里的任务。 */
async function deleteNodeFlow(kind: NodeKind, id: number, label: string): Promise<void> {
  const noun = kind === "module" ? "模块" : "子模块";
  try {
    const plan = await api.planDeleteNode(kind, id);
    const body =
      plan.taskCount > 0
        ? `${noun}「${label}」和其中的 ${plan.taskCount} 个任务会被删除，无法撤销。`
        : `${noun}「${label}」会被删除，无法撤销。`;

    const confirmed = await askConfirm({ title: `删除${noun}？`, body, confirmText: "删除" });
    if (!confirmed) return;

    // 正停在被删的子树里，就把范围收回「全部」——不能停在一个已经不存在的地方。
    const scopeWasInside = scopeInsideNode(kind, id);
    const submoduleIds =
      kind === "module" ? (state.modules.find((m) => m.id === id)?.submodules ?? []) : [];
    await api.deleteNode(kind, id);
    state.collapsedNodes.delete(id);
    for (const submodule of submoduleIds) state.collapsedNodes.delete(submodule.id);
    if (scopeWasInside) state.scope = { kind: "project" };

    await reload();
    showNotice(`已删除${noun}「${label}」`);
  } catch (error) {
    showNotice(describe(error), "error");
  }
}

function scopeInsideNode(kind: NodeKind, id: number): boolean {
  if (kind === "submodule") {
    return state.scope.kind === "submodule" && state.scope.id === id;
  }
  if (state.scope.kind === "module") return state.scope.id === id;
  if (state.scope.kind === "submodule") {
    const scopeId = state.scope.id;
    return state.modules.some(
      (module) => module.id === id && module.submodules.some((sub) => sub.id === scopeId),
    );
  }
  return false;
}

/** 删除项目：项目里的任务一并删除；正停在这个项目就回落到「我的待办」。 */
async function deleteProjectFlow(project: Project): Promise<void> {
  if (project.id === null) return;
  const id = project.id;

  try {
    const plan = await api.planDeleteProject(id);
    const body =
      plan.taskCount > 0
        ? `项目「${project.name}」和其中的 ${plan.taskCount} 个任务会被删除，无法撤销。`
        : `项目「${project.name}」会被删除，无法撤销。`;

    const confirmed = await askConfirm({ title: "删除项目？", body, confirmText: "删除" });
    if (!confirmed) return;

    const wasHere = state.container.kind === "project" && state.container.id === id;
    const wasCurrentProject = state.projectId === id;
    await api.deleteProject(id);

    if (wasHere || wasCurrentProject) {
      if (wasHere) state.container = { kind: "todo" };
      if (wasCurrentProject) state.projectId = null;
      state.scope = { kind: "project" };
      state.collapsedNodes.clear();
      state.collapsedGroups.clear();
      state.collapsedTasks.clear();
      state.addingSubtaskFor = null;
      subtaskDraft = "";
      try {
        localStorage.removeItem(PROJECT_STORAGE_KEY);
      } catch {
        // 忽略：存储不可用不影响删除本身。
      }
    }

    await reload();
    showNotice(`已删除项目「${project.name}」`);
  } catch (error) {
    showNotice(describe(error), "error");
  }
}

/** 仅开发版：删除活库并重建。正式版没有按钮，后端也不注册命令。 */
async function resetDatabaseFlow(): Promise<void> {
  const confirmed = await askConfirm({
    title: "重置数据库？",
    body: "当前数据库文件与全部数据会被删除，随后建一个空库。此操作仅开发版本可用，无法撤销。",
    confirmText: "删除并重建",
  });
  if (!confirmed) return;

  try {
    await api.resetDatabase();
    // 库已经换了一份：视图回到「我的待办」，旧的分组/折叠键全部失效。
    state.container = { kind: "todo" };
    state.projectId = null;
    state.scope = { kind: "project" };
    state.collapsedNodes.clear();
    state.collapsedGroups.clear();
    state.collapsedTasks.clear();
    state.addingSubtaskFor = null;
    subtaskDraft = "";
    try {
      localStorage.removeItem(PROJECT_STORAGE_KEY);
    } catch {
      // 忽略：重置数据库不依赖本地偏好清理成功。
    }
    closeMenu();
    await reload();
    showNotice("数据库已重置");
  } catch (error) {
    showNotice(describe(error), "error");
  }
}

async function createTask(): Promise<void> {
  const title = dom.taskInput.value.trim();
  if (!title) return;
  try {
    // 新任务跟着当前范围走：选模块就进模块，选子模块就进子模块。
    const target = resolveTarget(buildGroups(state.tasks, state.modules), state.scope);
    const created = await api.createTask(state.container, target, title);
    dom.taskInput.value = "";
    await reload();
    for (const notice of created.notices) showNotice(notice);
  } catch (error) {
    showNotice(describe(error), "error");
  }
}

// ---------------------------------------------------------------------------
// 轻提示
// ---------------------------------------------------------------------------

interface NoticeItem {
  message: string;
  kind: "info" | "error";
}

const noticeQueue: NoticeItem[] = [];
let noticeActive = false;
let noticeTimer: number | undefined;

/**
 * 底部一条会自己消失的提示，不阻塞任何操作。
 *
 * 多条提示按 FIFO 排队，一次只显示一条——auto 级联可能一次带回好几条，
 * 直接覆盖会让前一条永远看不见。
 */
function showNotice(message: string, kind: "info" | "error" = "info"): void {
  noticeQueue.push({ message, kind });
  if (!noticeActive) drainNotices();
}

function drainNotices(): void {
  const next = noticeQueue.shift();
  if (!next) {
    noticeActive = false;
    dom.notice.hidden = true;
    return;
  }

  noticeActive = true;
  dom.notice.textContent = next.message;
  dom.notice.dataset.kind = next.kind;
  dom.notice.hidden = false;

  window.clearTimeout(noticeTimer);
  noticeTimer = window.setTimeout(
    () => {
      noticeActive = false;
      drainNotices();
    },
    2000,
  );
}

/** 命令失败时 Tauri 抛的是后端那个 `String`，这里只负责把它变成一句话。 */
function describe(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return String(error);
}

// ---------------------------------------------------------------------------
// 启动
// ---------------------------------------------------------------------------

// 模块脚本通常早于 DOMContentLoaded 执行，但如果它被一个带 await 的入口 import，
// 事件可能已经闪过去了。两种时机都要能起来。
if (document.readyState === "loading") {
  window.addEventListener("DOMContentLoaded", boot);
} else {
  boot();
}

/**
 * 数据库起不来时的独立页面：只显示后端给的原因与备份路径，不碰任何命令。
 * 后端在 setup 里把主窗口换成 `index.html?startupError=...`，这里直接接管。
 */
function renderStartupError(message: string): void {
  document.title = "无法启动 — Knot";
  document.body.classList.add("startup-error-body");

  const panel = el("main", "startup-error");
  const card = el("section", "startup-error-card");
  card.append(
    el("h1", "startup-error-title", "无法启动"),
    el(
      "p",
      "startup-error-lead",
      "数据库迁移没有完成。程序没有继续加载，数据库文件没有被部分改写。",
    ),
    el("pre", "startup-error-detail", message),
    el(
      "p",
      "startup-error-hint",
      "按照上面的原因处理后再打开应用。需要恢复时，用上面列出的备份文件覆盖 tasks.sqlite。",
    ),
  );
  panel.append(card);
  document.body.replaceChildren(panel);
}

function boot(): void {
  const startupError = new URLSearchParams(location.search).get(STARTUP_ERROR_PARAM);
  if (startupError) {
    renderStartupError(startupError);
    return;
  }
  // 原生右键菜单在应用里没有用处；项目与范围节点会各自 preventDefault
  // 并打开自己的菜单，这里统一兜底禁用其余位置的浏览器菜单。
  document.addEventListener("contextmenu", (event) => {
    event.preventDefault();
  });
  initTheme();
  void initWindowControls();
  // 拖拽区会先被 Tauri 的拖动逻辑拦截，菜单的全局 mousedown 收不到；这里用捕获阶段先收起。
  dom.titlebarDrag.addEventListener("mousedown", () => closeMenu(), true);
  dom.projectBtn.addEventListener("click", openProjectMenu);
  dom.aboutBtn.addEventListener("click", openAboutDialog);
  initTooltips();
  initColumnResizing();
  render();
  void reload();
}

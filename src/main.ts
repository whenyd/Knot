/**
 * View layer: state + events + rendering. Native DOM, no UI framework.
 *
 * Data flows one way only:
 *   `list_projects` / `list_modules` / `list_tasks`  →  pure sorting/grouping functions (grouping.ts)  →  DOM
 * There is no reverse binding: the UI always "re-fetches everything, then redraws in full". The data
 * volume is small, and a full redraw costs far less than maintaining incremental patches.
 */
import {
  api,
  type BackendMessage,
  type Container,
  type Module,
  type MoveTarget,
  type NodeKind,
  type ParentStatusProposal,
  type Priority,
  type Project,
  type ScopeTarget,
  type Status,
  type Task,
} from "./api";
import { backendMessageKey, describe } from "./errors";
import { formatSchedule } from "./i18n/date";
import {
  initI18n,
  readLanguageMode,
  setLanguageMode,
  t,
  tp,
  type LanguageMode,
  type MessageKey,
} from "./i18n";
import { translateBackendMessage } from "./i18n/backend";
import { applyStaticTranslations } from "./i18n/static";
import {
  closeMenu,
  openMenu,
  type MenuItem,
  type MenuOptions,
  type MenuPosition,
} from "./menu";
import {
  buildGroups,
  countGroup,
  countGroups,
  filterTasks,
  resolveTarget,
  selectScope,
  sortParentCandidates,
  sortTasks,
  taskMatchesFilters,
  type Group,
  type Scope,
  type SortField,
  type TaskFilters,
  type TaskSort,
} from "./grouping";
import { initTooltips } from "./tooltip";
import appIcon from "../src-tauri/icons/icon.png";

/** When the database fails to open, the backend hands the reason to the frontend through this query param. */
const STARTUP_ERROR_PARAM = "startupError";

/** Kept in sync with the version in src-tauri/tauri.conf.json. */
const APP_VERSION = "0.2.0";

// Three-state status: clicking the status cell cycles to the next state; the right-click "Set status…" menu picks directly, and cycling is just a shortcut.
const NEXT_STATUS: Record<Status, Status> = { todo: "doing", doing: "done", done: "todo" };
const STATUS_KEYS: Record<Status, MessageKey> = {
  todo: "status.todo",
  doing: "status.doing",
  done: "status.done",
};
const STATUS_ORDER: Status[] = ["todo", "doing", "done"];

const statusLabel = (status: Status): string => t(STATUS_KEYS[status]);

// Priority is picked from a popover: all three levels are laid out, no cycling to find one.
const PRIORITY_ORDER: Priority[] = ["high", "mid", "low"];
const PRIORITY_KEYS: Record<Priority, MessageKey> = {
  high: "priority.high",
  mid: "priority.mid",
  low: "priority.low",
};

const priorityLabel = (priority: Priority): string => t(PRIORITY_KEYS[priority]);

/** The three time columns share one set of copy: creation time is written on creation; start / end follow status changes. */
type TimeField = "created" | "start" | "end";
const TIME_LABEL_KEYS: Record<TimeField, MessageKey> = {
  created: "time.createdLabel",
  start: "time.startLabel",
  end: "time.endLabel",
};
const TIME_HINT_KEYS: Record<TimeField, MessageKey> = {
  created: "time.createdHint",
  start: "time.stateHint",
  end: "time.stateHint",
};

interface State {
  projects: Project[];
  container: Container;
  /** Current project context; kept after switching to "My tasks", so the sidebar project name and module tree stay visible. */
  projectId: number | null;
  scope: Scope;
  modules: Module[];
  tasks: Task[];
  /** All tasks in the current project context, for sidebar module-tree counts; decoupled from the table container. */
  projectTasks: Task[];
  /** Module ids **collapsed** in the sidebar scope tree. Expanded by default, so collapsed ones are stored. */
  collapsedNodes: Set<number>;
  /** Classification groups collapsed in the table; keys come from `groupKey`. Independent of sidebar collapsing. */
  collapsedGroups: Set<string>;
  /** Parent task id whose inline subtask input is open; only one at a time. */
  addingSubtaskFor: number | null;
  /** Parent task ids whose subtasks are collapsed in the table. Expanded by default, so collapsed ones are stored. */
  collapsedTasks: Set<number>;
  /** Top-bar filters persist for this session and reset neither on scope nor on project switch. */
  statusFilter: Set<Status>;
  priorityFilter: Set<Priority>;
  /** Header column sort; null = default (status → priority → creation time). */
  sort: TaskSort | null;
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
  sort: null,
};

/** Draft of the inline subtask input: kept outside the state object, so unrelated redraws do not lose typing. */
let subtaskDraft = "";

/** Choices for the next quick-added task only. Reset to the scope defaults when the view changes or after adding. */
let composer: { target: ScopeTarget; priority: Priority } = {
  target: { moduleId: null, submoduleId: null },
  priority: "mid",
};

/** Double-click a scope node to rename: the first click redraws the sidebar and replaces the element, so native dblclick is unreliable;
    instead detect "same node clicked again within 400ms"; state is module-level so redraws do not lose it. */
let scopeRenameTap: { key: string; at: number } | null = null;

/** Sidebar module drag-sort: pointerdown only records the start point; dragging begins past a threshold
    so it does not steal events from "click to switch scope". Only one module drags at a time. */
interface ModuleDrag {
  id: number;
  row: HTMLElement;
  startX: number;
  startY: number;
  active: boolean;
  /** Insertion position: index in the module-row sequence; null = stay in place (no reorder). */
  insertion: number | null;
}
let moduleDrag: ModuleDrag | null = null;
const MODULE_DRAG_THRESHOLD = 4;

/** Input session number: stale blur callbacks only recognize their own round, so clicking "+" cannot commit the previous round. */
let subtaskInputSession = 0;
/** Restore the last project only on first load; later reloads must not override the user's current choice. */
let startupProjectRestored = false;

// ---------------------------------------------------------------------------
// DOM
// ---------------------------------------------------------------------------

function must<T extends HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (!node) throw new Error(`Missing DOM node #${id}`);
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

/** Reuse symbols from index.html; sizing is left to CSS. */
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
  sidebar: must<HTMLElement>("sidebar"),
  sidebarResizer: must<HTMLElement>("sidebar-resizer"),
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
  gridScroll: must<HTMLElement>("grid-scroll"),
  body: must<HTMLTableSectionElement>("grid-body"),
  empty: must<HTMLParagraphElement>("empty"),
  taskForm: must<HTMLFormElement>("task-form"),
  taskInput: must<HTMLInputElement>("task-input"),
  taskModule: must<HTMLButtonElement>("task-module"),
  taskModuleLabel: must<HTMLElement>("task-module-label"),
  taskPriority: must<HTMLButtonElement>("task-priority"),
  taskPriorityLabel: must<HTMLElement>("task-priority-label"),
  statusCount: must<HTMLElement>("status-count"),
  languageBtn: must<HTMLButtonElement>("language-btn"),
  languageLabel: must<HTMLElement>("language-label"),
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

// Database reset appears in dev builds only; in release builds the whole branch is dropped and the button stays hidden.
if (import.meta.env.DEV) {
  dom.devReset.hidden = false;
  dom.devReset.addEventListener("click", () => void resetDatabaseFlow());
}

// ---------------------------------------------------------------------------
// Theme and current project
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

function refreshThemeLabel(): void {
  const mode = readThemeMode();
  const iconName = mode === "light" ? "sun" : mode === "dark" ? "moon" : "monitor";
  dom.themeBtn.querySelector("use")?.setAttribute("href", `#i-${iconName}`);
  dom.themeLabel.textContent = t(
    mode === "light" ? "theme.light" : mode === "dark" ? "theme.dark" : "theme.system",
  );
}

function applyTheme(mode: ThemeMode): void {
  const prefersDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
  const resolved = mode === "system" ? (prefersDark ? "dark" : "light") : mode;
  document.documentElement.dataset.theme = resolved;
  try {
    localStorage.setItem(THEME_STORAGE_KEY, mode);
  } catch {
    // Storage failure only affects the preference next launch; it does not block theme switching.
  }
  refreshThemeLabel();
}

function initTheme(): void {
  applyTheme(readThemeMode());
  dom.themeBtn.addEventListener("click", () => {
    const mode = readThemeMode();
    openMenu({
      items: [
        { label: t("theme.light"), checked: mode === "light", onSelect: () => applyTheme("light") },
        { label: t("theme.dark"), checked: mode === "dark", onSelect: () => applyTheme("dark") },
        {
          label: t("theme.system"),
          checked: mode === "system",
          onSelect: () => applyTheme("system"),
        },
      ],
      position: { kind: "anchor", element: dom.themeBtn },
      label: t("theme.label"),
    });
  });
  window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
    if (readThemeMode() === "system") applyTheme("system");
  });
}

// ---------------------------------------------------------------------------
// Language
// ---------------------------------------------------------------------------

let languageMode: LanguageMode = "system";

function refreshLanguageButton(): void {
  dom.languageLabel.textContent =
    languageMode === "zh"
      ? t("language.name.zh")
      : languageMode === "en"
        ? t("language.name.en")
        : t("language.mode.system");
}

function initLanguage(): void {
  initI18n();
  languageMode = readLanguageMode();
  refreshLanguageButton();
  dom.languageBtn.addEventListener("click", () => {
    openMenu({
      items: [
        {
          label: t("language.name.zh"),
          checked: languageMode === "zh",
          onSelect: () => applyLanguageMode("zh"),
        },
        {
          label: t("language.name.en"),
          checked: languageMode === "en",
          onSelect: () => applyLanguageMode("en"),
        },
        {
          label: t("language.mode.system"),
          checked: languageMode === "system",
          onSelect: () => applyLanguageMode("system"),
        },
      ],
      position: { kind: "anchor", element: dom.languageBtn },
      label: t("language.label"),
    });
  });
}

function applyLanguageMode(mode: LanguageMode): void {
  languageMode = mode;
  const changed = setLanguageMode(mode);
  refreshLanguageButton();
  if (!changed) return;
  closeMenu();
  applyStaticTranslations();
  refreshThemeLabel();
  refreshWindowMaxLabel();
  // Re-clamp cached column widths under the new language's minimums.
  fixedWidths = null;
  render();
}

type AppPlatform = "windows" | "macos" | "linux";

/** macOS uses native traffic lights; Windows uses custom buttons in the top bar; Linux keeps the system title bar. */
function detectPlatform(): AppPlatform {
  const ua = navigator.userAgent;
  if (/Macintosh|Mac OS X/i.test(ua)) return "macos";
  if (/Linux/i.test(ua) && !/Android/i.test(ua)) return "linux";
  return "windows";
}

let windowMaximized = false;
let windowControlsReady = false;

/** Keeps the maximize button label correct after a language switch. */
function refreshWindowMaxLabel(): void {
  if (!windowControlsReady) return;
  const label = t(windowMaximized ? "window.restore" : "window.maximize");
  dom.windowMax.title = label;
  dom.windowMax.setAttribute("aria-label", label);
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
      windowMaximized = await appWindow.isMaximized();
      maxIcon?.setAttribute("href", windowMaximized ? "#i-restore" : "#i-maximize");
      windowControlsReady = true;
      refreshWindowMaxLabel();
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
    // Running Vite in a browser has no Tauri runtime, so the buttons are static previews only.
    console.warn("Window controls failed to initialize", error);
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

/** Remember the current project context; "My tasks" is not a project, so it is not written. */
function persistCurrentProject(): void {
  try {
    if (state.projectId !== null) {
      localStorage.setItem(PROJECT_STORAGE_KEY, String(state.projectId));
    } else {
      localStorage.removeItem(PROJECT_STORAGE_KEY);
    }
  } catch {
    // When storage is unavailable, fall back to this session's navigation behavior.
  }
}

function deleteCurrentProject(): void {
  const project = currentProject();
  if (!project || project.isTodo) {
    showNotice(t("nav.todoCannotDelete"));
    return;
  }
  void deleteProjectFlow(project);
}

function openProjectMenu(): void {
  const items: MenuItem[] = [
    { label: t("nav.newProject"), onSelect: () => void createProjectFlow() },
    { label: t("nav.deleteProject"), onSelect: deleteCurrentProject },
    { type: "separator", label: "" },
    { type: "header", label: t("nav.allProjects") },
    ...state.projects
      .filter((project) => !project.isTodo)
      .map((project) => ({
        label: projectName(project),
        checked: isCurrent(project),
        onSelect: () => void selectProject(project),
      })),
  ];

  openMenu({
    items,
    position: { kind: "anchor", element: dom.projectBtn },
    label: t("nav.projects"),
    className: "is-project",
  });
}

/** Keep the right-clicked sidebar row highlighted while its context menu is open. */
function openSidebarMenu(target: HTMLElement, options: MenuOptions): void {
  openMenu({
    ...options,
    onClose: () => target.classList.remove("is-menu-open"),
  });
  // Add after openMenu: closing a previous menu would otherwise remove the class.
  target.classList.add("is-menu-open");
}

// ---------------------------------------------------------------------------
// Column widths
// ---------------------------------------------------------------------------

type ResizableColumn = "status" | "title" | "created" | "start" | "end" | "priority" | "note";
type FixedColumn = Exclude<ResizableColumn, "title">;

/** The "Task" column takes the remaining width; other columns use user-dragged or default widths. */
const FIXED_COLUMNS: readonly FixedColumn[] = [
  "status",
  "priority",
  "created",
  "start",
  "end",
  "note",
];
const COLUMN_MIN: Record<ResizableColumn, number> = {
  status: 56,
  title: 100,
  created: 80,
  start: 80,
  end: 80,
  priority: 84,
  note: 80,
};
const COLUMN_KEYS: Record<ResizableColumn, MessageKey> = {
  status: "column.status",
  title: "column.title",
  created: "column.created",
  start: "column.start",
  end: "column.end",
  priority: "column.priority",
  note: "column.note",
};
/** Matches the col / th order in index.html; dragging searches for space on both sides in this order. */
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
    // Storage unavailable or corrupt: fall back to default column widths without affecting this session.
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

/**
 * Fixed columns take their current widths and the "Task" column fills the rest; when the window narrows the task column stays at its minimum.
 *
 * Available width comes from the scroll container, not the table itself: the table width is driven by column widths, so reading `grid.clientWidth`
 * creates the loop "table won't shrink → task column won't shrink → table won't shrink", leaving only horizontal scrolling when the window shrinks.
 */
function applyColumnWidths(): void {
  const widths = currentFixedWidths();
  const fixedTotal = FIXED_COLUMNS.reduce((sum, key) => sum + widths[key], 0);
  titleColumnWidth = Math.max(COLUMN_MIN.title, dom.gridScroll.clientWidth - fixedTotal);

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
    // Only affects whether this is remembered next time; this drag has already taken effect.
  }
}

function columnWidthOf(key: ResizableColumn, widths: Record<FixedColumn, number>): number {
  return key === "title" ? titleColumnWidth : widths[key as FixedColumn];
}

/** Current display widths of all columns; snapshot once when a drag starts, and compute later movement from it. */
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
 * Distribute one horizontal movement across the two sides of a boundary:
 * - Drag right: the right side yields space from near to far until its minimum; the left gains only what the right actually yielded.
 * - Drag left: the left side yields space from near to far; the right gains only what the left actually yielded.
 * So when one side is already at its minimum, columns farther on that side move too instead of the line getting stuck.
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
  dom.resizeTip.textContent = t("column.resizeTip", {
    left: t(COLUMN_KEYS[left]),
    leftWidth,
    right: t(COLUMN_KEYS[right]),
    rightWidth,
  });
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
 * Every header boundary is a drag handle: dragging changes the columns on both sides at once, keeping total width constant;
 * past a side's minimum width, further dragging moves columns farther out on that side in column order.
 * On hover both headers light up; while dragging, both current widths float up, avoiding the illusion that "only one column moved".
 */
function initColumnResizing(): void {
  applyColumnWidths();
  for (const handle of dom.grid.querySelectorAll<HTMLElement>(".col-resizer")) {
    const boundary = handle.dataset.boundary?.split(":");
    if (!boundary || boundary.length !== 2) continue;
    const [left, right] = boundary as [ResizableColumn, ResizableColumn];
    handle.title = t("column.resizeHandle");
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
// Sidebar width
// ---------------------------------------------------------------------------

const SIDEBAR_STORAGE_KEY = "knot.sidebarWidth.v1";
const SIDEBAR_DEFAULT_WIDTH = 280;
const SIDEBAR_MIN_WIDTH = 180;
const SIDEBAR_MAX_WIDTH = 420;

/** Leave at least 360px for the main area; the maximum shrinks as the window narrows. */
function sidebarMaxWidth(): number {
  return Math.max(SIDEBAR_MIN_WIDTH, Math.min(SIDEBAR_MAX_WIDTH, window.innerWidth - 360));
}

/** Apply a width and return the actual clamped value. */
function applySidebarWidth(width: number): number {
  const clamped = Math.round(
    Math.min(sidebarMaxWidth(), Math.max(SIDEBAR_MIN_WIDTH, width)),
  );
  dom.sidebar.style.flex = `0 0 ${clamped}px`;
  dom.sidebarResizer.setAttribute("aria-valuemin", String(SIDEBAR_MIN_WIDTH));
  dom.sidebarResizer.setAttribute("aria-valuemax", String(sidebarMaxWidth()));
  dom.sidebarResizer.setAttribute("aria-valuenow", String(clamped));
  // A wider / narrower sidebar changes the table's available width, so the task column must be recomputed too.
  applyColumnWidths();
  return clamped;
}

function saveSidebarWidth(width: number): void {
  try {
    localStorage.setItem(SIDEBAR_STORAGE_KEY, String(width));
  } catch {
    // If it cannot be stored, it applies to this session only.
  }
}

function readSidebarWidth(): number {
  try {
    const raw = localStorage.getItem(SIDEBAR_STORAGE_KEY);
    const value = raw === null ? Number.NaN : Number.parseFloat(raw);
    return Number.isFinite(value) ? value : SIDEBAR_DEFAULT_WIDTH;
  } catch {
    return SIDEBAR_DEFAULT_WIDTH;
  }
}

function beginSidebarResize(event: PointerEvent): void {
  if (event.button !== 0) return;
  event.preventDefault();

  const startX = event.clientX;
  const startWidth = dom.sidebar.getBoundingClientRect().width;
  let lastWidth = startWidth;

  const onMove = (moveEvent: PointerEvent): void => {
    lastWidth = applySidebarWidth(startWidth + moveEvent.clientX - startX);
  };

  const finish = (): void => {
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", finish);
    window.removeEventListener("pointercancel", finish);
    document.body.classList.remove("col-resizing");
    dom.sidebarResizer.classList.remove("is-active");
    saveSidebarWidth(lastWidth);
  };

  dom.sidebarResizer.classList.add("is-active");
  document.body.classList.add("col-resizing");
  window.addEventListener("pointermove", onMove);
  window.addEventListener("pointerup", finish);
  window.addEventListener("pointercancel", finish);
}

function initSidebarResizing(): void {
  applySidebarWidth(readSidebarWidth());
  dom.sidebarResizer.addEventListener("pointerdown", beginSidebarResize);
  dom.sidebarResizer.addEventListener("keydown", (event) => {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault();
    const delta = event.key === "ArrowRight" ? 16 : -16;
    const width = applySidebarWidth(dom.sidebar.getBoundingClientRect().width + delta);
    saveSidebarWidth(width);
  });
  window.addEventListener("resize", () => {
    applySidebarWidth(dom.sidebar.getBoundingClientRect().width);
  });
}

// ---------------------------------------------------------------------------
// Column sorting
// ---------------------------------------------------------------------------

const SORTABLE_FIELDS: readonly SortField[] = ["priority", "created", "start", "end"];

const SORT_KEYS: Record<SortField, MessageKey> = {
  priority: "sort.priority",
  created: "sort.created",
  start: "sort.start",
  end: "sort.end",
};

/** Button right of a column name: ascending → descending → restore default order. */
function cycleColumnSort(field: SortField): void {
  const current = state.sort;
  if (current === null || current.field !== field) {
    state.sort = { field, direction: "asc" };
  } else if (current.direction === "asc") {
    state.sort = { field, direction: "desc" };
  } else {
    state.sort = null;
  }
  render();
}

function initColumnSorting(): void {
  for (const button of dom.grid.querySelectorAll<HTMLButtonElement>(".sort-btn")) {
    const field = button.dataset.sort as SortField | undefined;
    if (field === undefined || !SORTABLE_FIELDS.includes(field)) continue;
    button.addEventListener("click", () => cycleColumnSort(field));
  }
}

/** Arrow, tooltip and `aria-sort` follow state.sort. */
function renderSortIndicators(): void {
  for (const button of dom.grid.querySelectorAll<HTMLButtonElement>(".sort-btn")) {
    const field = button.dataset.sort as SortField | undefined;
    if (field === undefined) continue;

    const direction = state.sort?.field === field ? state.sort.direction : null;
    const label = t(SORT_KEYS[field]);
    button.textContent = direction === "asc" ? "▲" : direction === "desc" ? "▼" : "⇅";
    button.classList.toggle("is-active", direction !== null);

    const hint =
      direction === "asc"
        ? t("sort.ascHint", { label })
        : direction === "desc"
          ? t("sort.descHint", { label })
          : t("sort.defaultHint", { label });
    button.title = hint;
    button.setAttribute("aria-label", hint);
    button
      .closest("th")
      ?.setAttribute(
        "aria-sort",
        direction === "asc" ? "ascending" : direction === "desc" ? "descending" : "none",
      );
  }
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

/** Fetch all data for the current container and redraw. Called after every write. */
async function reload(): Promise<void> {
  try {
    state.projects = await api.listProjects();

    // Restore the last project only on first load; later reloads must respect the user's current choice.
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
            // Ignore: an invalid value does not block startup.
          }
        }
      }
    }

    // Take local copies: the user may switch containers between awaits, so one redraw honors one container.
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
  // Switching to "My tasks" does not clear the project context: the sidebar project name and module tree stay visible.
  if (!isTodo) state.projectId = project.id;
  // A different container is a different tree: reset the scope to "whole project"; collapse state no longer applies.
  state.scope = { kind: "project" };
  resetComposer();
  state.collapsedNodes.clear();
  state.collapsedGroups.clear();
  state.collapsedTasks.clear();
  state.addingSubtaskFor = null;
  subtaskDraft = "";
  persistCurrentProject();
  closeMenu();
  await reload();
}

/** If the parent is gone (deleted or container switched), retract the inline input instead of staying on a nonexistent parent. */
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
// Rendering
// ---------------------------------------------------------------------------

function render(): void {
  // The table uses the current container's tasks; the sidebar tree always shows the project context (kept when switching to "My tasks").
  const groups = buildGroups(state.tasks, state.modules, state.sort);
  const sidebarGroups =
    state.projectId !== null ? buildGroups(state.projectTasks, state.modules) : groups;
  renderSidebar(sidebarGroups);
  renderToolbar(groups);
  renderTable(groups);
  renderSortIndicators();
  // Changing data volume may add / remove the vertical scrollbar, so the task column's remaining width must be recomputed.
  applyColumnWidths();
}

function renderSidebar(groups: Group[]): void {
  dom.todoSection.replaceChildren();

  const todo = state.projects.find((project) => project.isTodo);
  if (todo) dom.todoSection.append(renderSideItem(todo));

  const project = currentProject();
  const hasProject = state.projectId !== null && project !== undefined && !project.isTodo;
  dom.projectHeading.textContent = hasProject ? projectName(project) : "";
  dom.projectHeading.hidden = !hasProject;

  renderScopeTree(groups);
}

function renderSideItem(project: Project): HTMLButtonElement {
  const button = el("button", "side-item");
  button.type = "button";
  button.classList.toggle("selected", isCurrent(project));
  button.title = projectName(project);

  button.append(
    el("span", "side-name", projectName(project)),
    el("span", "side-count", String(project.taskCount)),
  );
  button.addEventListener("click", () => {
    void selectProject(project);
  });

  // Right-click: do not open a menu when the project has no structure to manage ("My tasks" is one such case).
  // The native context menu is disabled globally by the listener in boot().
  button.addEventListener("contextmenu", (event) => {
    if (project.isTodo || project.id === null) return;
    const projectId = project.id;
    event.preventDefault();
    openSidebarMenu(button, {
      items: [
        { label: t("nav.newModule"), onSelect: () => void createModuleIn(projectId) },
        {
          label: t("nav.renameProject"),
          onSelect: () => renameNode({ kind: "project", id: projectId, name: project.name }),
        },
        { label: t("nav.deleteProject"), onSelect: () => void deleteProjectFlow(project) },
      ],
      position: { kind: "point", x: event.clientX, y: event.clientY },
    });
  });

  return button;
}

/** Sidebar current project name: context menu (new module / rename / delete) and double-click rename. */
function initProjectHeading(): void {
  dom.projectHeading.addEventListener("dblclick", () => {
    const project = currentProject();
    if (!project || project.isTodo || project.id === null) return;
    renameNode({ kind: "project", id: project.id, name: project.name });
  });

  dom.projectHeading.addEventListener("contextmenu", (event) => {
    // While renaming in place, do not steal the input's right-click.
    if (event.target instanceof HTMLInputElement) return;
    const project = currentProject();
    if (!project || project.isTodo || project.id === null) return;
    event.preventDefault();
    const projectId = project.id;
    openSidebarMenu(dom.projectHeading, {
      items: [
        { label: t("nav.newModule"), onSelect: () => void createModuleIn(projectId) },
        {
          label: t("nav.renameProject"),
          onSelect: () => renameNode({ kind: "project", id: projectId, name: project.name }),
        },
        { label: t("nav.deleteProject"), onSelect: () => void deleteProjectFlow(project) },
      ],
      position: { kind: "point", x: event.clientX, y: event.clientY },
    });
  });
}

/**
 * Right-click on sidebar whitespace: also creates a module in the current project.
 * Project rows / scope rows / the dev button handle their own right-clicks (already preventDefault), so this only covers empty areas.
 */
function initSidebarContextMenu(): void {
  dom.sidebar.addEventListener("contextmenu", (event) => {
    if (event.defaultPrevented) return;
    const projectId = state.projectId;
    if (projectId === null) return;
    if (
      event.target instanceof Element &&
      event.target.closest(".side-item, .scope-row, button, input")
    ) {
      return;
    }
    event.preventDefault();
    openMenu({
      items: [{ label: t("nav.newModule"), onSelect: () => void createModuleIn(projectId) }],
      position: { kind: "point", x: event.clientX, y: event.clientY },
    });
  });
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

/** The virtual todo container has no stored name; render its label from `isTodo`. */
function projectName(project: Project): string {
  return project.isTodo ? t("nav.todo") : project.name;
}

/** Display name of a scope; the derived default group has no stored label. */
function scopeLabel(groups: readonly Group[], scope: Scope): string {
  switch (scope.kind) {
    case "project":
      return t("scope.all");
    case "unclassified":
      return t("scope.default");
    case "module":
      return (
        groups.find((group) => group.kind === "module" && group.id === scope.id)?.label ??
        t("scope.module")
      );
    case "submodule": {
      for (const group of groups) {
        const child = group.children.find(
          (node) => node.kind === "submodule" && node.id === scope.id,
        );
        if (child) return child.label ?? t("scope.submodule");
      }
      return t("scope.submodule");
    }
  }
}

/**
 * Sidebar scope tree: whole project / module / submodule / default. Clicking one limits the main view to that subtree.
 * "My tasks" has no modules, so it has no such tree.
 */
function renderScopeTree(groups: Group[]): void {
  dom.scopeSection.replaceChildren();

  const visible = state.projectId !== null;
  dom.scopeSection.hidden = !visible;
  if (!visible) return;

  dom.scopeSection.append(
    renderScopeNode({
      scope: { kind: "project" },
      label: t("scope.all"),
      count: countGroups(groups),
      indent: 0,
    }),
  );

  // "Default" comes before modules, matching table group order; an empty default group takes no row.
  const unclassified = groups.find((group) => group.kind === "unclassified");
  if (unclassified) {
    dom.scopeSection.append(
      renderScopeNode({
        scope: { kind: "unclassified" },
        label: unclassified.label ?? t("scope.default"),
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
        label: group.label ?? t("scope.module"),
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
          label: child.label ?? t("scope.submodule"),
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
  /** Only module nodes with submodules get a collapse caret */
  collapsible?: { id: number; collapsed: boolean };
}

function renderScopeNode(options: ScopeNodeOptions): HTMLElement {
  const row = el("div", "scope-row");
  row.classList.toggle("selected", isScopeCurrent(options.scope));
  row.style.paddingLeft = `${8 + options.indent * 16}px`;

  if (options.collapsible) {
    const { id, collapsed } = options.collapsible;
    // The triangle is drawn by CSS ::before; put no character in the element:
    // a literal ▾ plus the pseudo-element makes two triangles (already got this wrong once).
    const caret = el("button", collapsed ? "scope-caret is-collapsed has-icon" : "scope-caret has-icon");
    caret.type = "button";
    caret.title = t(collapsed ? "common.expand" : "common.collapse");
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
  if (options.scope.kind === "module" || options.scope.kind === "submodule") {
    name.dataset.kind = options.scope.kind;
    name.dataset.id = String(options.scope.id);
  }
  // Double-clicking a module / submodule name renames it in place; "All" and "Default" are virtual scopes with no name to edit.
  const scope = options.scope;
  if (scope.kind === "module") {
    // Module rows can be dragged up/down to reorder; submodules move with their module and are not dragged alone.
    row.dataset.moduleId = String(scope.id);
    row.addEventListener("pointerdown", (event) =>
      beginModuleDrag(event, scope.id, row),
    );
  }
  name.addEventListener("click", () => {
    if (scope.kind === "module" || scope.kind === "submodule") {
      const key = `${scope.kind}:${scope.id}`;
      const now = Date.now();
      if (scopeRenameTap?.key === key && now - scopeRenameTap.at < 400) {
        scopeRenameTap = null;
        renameNode({ kind: scope.kind, id: scope.id, name: options.label });
        return;
      }
      scopeRenameTap = { key, at: now };
    }
    applyScope(scope);
  });

  row.append(name, el("span", "scope-count", `${options.count.done}/${options.count.total}`));

  row.addEventListener("contextmenu", (event) => {
    // While renaming in place, do not steal the input's right-click.
    if (event.target instanceof HTMLInputElement) return;
    const items = scopeMenuItems(options);
    if (items.length === 0) return;
    event.preventDefault();
    openSidebarMenu(row, {
      items,
      position: { kind: "point", x: event.clientX, y: event.clientY },
    });
  });

  return row;
}

/** Currently rendered module rows, in sidebar top-to-bottom order. Submodule rows are not included. */
function moduleRows(): HTMLElement[] {
  return Array.from(dom.scopeSection.querySelectorAll<HTMLElement>(".scope-row[data-module-id]"));
}

function beginModuleDrag(event: PointerEvent, id: number, row: HTMLElement): void {
  if (event.button !== 0) return;
  // While renaming in place, let the input handle the pointer itself; right-click stays with the menu.
  if (event.target instanceof HTMLInputElement) return;
  if (moduleDrag !== null) return;

  moduleDrag = {
    id,
    row,
    startX: event.clientX,
    startY: event.clientY,
    active: false,
    insertion: null,
  };
  document.addEventListener("pointermove", onModuleDragMove, { passive: false });
  document.addEventListener("pointerup", endModuleDrag);
  document.addEventListener("pointercancel", endModuleDrag);
}

function onModuleDragMove(event: PointerEvent): void {
  const drag = moduleDrag;
  if (!drag) return;

  if (!drag.active) {
    const moved = Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY);
    if (moved < MODULE_DRAG_THRESHOLD) return;
    drag.active = true;
    drag.row.classList.add("is-dragging");
    document.body.classList.add("is-module-dragging");
  }

  event.preventDefault();
  autoScrollSidebar(event.clientY);
  updateModuleDropIndicator(moduleRows(), event.clientY);
}

/**
 * Which row the pointer falls before: compare module-row midlines one by one. Below the last row's midline is the end of the sequence.
 * Submodule rows do not participate; dropping on a submodule counts as its owning module's position.
 */
function moduleDropIndex(rows: readonly HTMLElement[], clientY: number): number {
  for (let index = 0; index < rows.length; index += 1) {
    const rect = rows[index].getBoundingClientRect();
    if (clientY < rect.top + rect.height / 2) return index;
  }
  return rows.length;
}

function updateModuleDropIndicator(rows: readonly HTMLElement[], clientY: number): void {
  const drag = moduleDrag;
  if (!drag) return;

  clearModuleDropIndicator();
  const dragIndex = rows.findIndex((row) => Number(row.dataset.moduleId) === drag.id);
  const insertion = moduleDropIndex(rows, clientY);

  // Inserting before itself or immediately after itself is no move: draw no line and write nothing.
  if (dragIndex !== -1 && (insertion === dragIndex || insertion === dragIndex + 1)) {
    drag.insertion = null;
    return;
  }

  drag.insertion = insertion;
  if (insertion < rows.length) {
    rows[insertion].classList.add("drop-before");
  } else if (rows.length > 0) {
    rows[rows.length - 1].classList.add("drop-after");
  }
}

function clearModuleDropIndicator(): void {
  for (const row of dom.scopeSection.querySelectorAll(".scope-row")) {
    row.classList.remove("drop-before", "drop-after");
  }
}

/** Keep scrolling when dragging to the sidebar's top/bottom edge, so many modules can still be dragged past. */
function autoScrollSidebar(clientY: number): void {
  const rect = dom.sidebar.getBoundingClientRect();
  const edge = 24;
  if (clientY < rect.top + edge) dom.sidebar.scrollTop -= 10;
  else if (clientY > rect.bottom - edge) dom.sidebar.scrollTop += 10;
}

function endModuleDrag(event: PointerEvent): void {
  const drag = moduleDrag;
  moduleDrag = null;
  document.removeEventListener("pointermove", onModuleDragMove);
  document.removeEventListener("pointerup", endModuleDrag);
  document.removeEventListener("pointercancel", endModuleDrag);
  if (!drag) return;

  clearModuleDropIndicator();
  drag.row.classList.remove("is-dragging");
  document.body.classList.remove("is-module-dragging");
  if (!drag.active) return;

  // After the drag the browser still fires a click; do not let it switch scope or collapse a group.
  swallowNextClick();

  // Gesture interrupted by the system (pointercancel): only clean up visuals, do not commit the order.
  if (event.type === "pointercancel") return;

  const insertion = drag.insertion;
  if (insertion === null) return;

  const rows = moduleRows();
  const next = insertion < rows.length ? rows[insertion] : undefined;
  const beforeId = next ? Number(next.dataset.moduleId) : null;
  void persistModuleOrder(drag.id, beforeId);
}

/** Swallow the one click that follows a finished drag (both sidebar row clicks and collapse carets). */
function swallowNextClick(): void {
  const handler = (event: MouseEvent): void => {
    event.preventDefault();
    event.stopPropagation();
    document.removeEventListener("click", handler, true);
    window.clearTimeout(timer);
  };
  const timer = window.setTimeout(() => {
    document.removeEventListener("click", handler, true);
  }, 250);
  document.addEventListener("click", handler, true);
}

async function persistModuleOrder(id: number, beforeId: number | null): Promise<void> {
  try {
    await api.moveModule(id, beforeId);
    await reload();
  } catch (error) {
    showNotice(describe(error), "error");
  }
}

/** What a right-click on a scope node can do. The menu lives at the level where the structure is created. */
function scopeMenuItems(options: ScopeNodeOptions): MenuItem[] {
  const { scope, label } = options;

  switch (scope.kind) {
    case "project": {
      const projectId = state.projectId;
      return projectId === null
        ? []
        : [{ label: t("nav.newModule"), onSelect: () => void createModuleIn(projectId) }];
    }
    case "module":
      return [
        { label: t("nav.newSubmodule"), onSelect: () => void createSubmoduleIn(scope.id) },
        {
          label: t("nav.renameModule"),
          onSelect: () => renameNode({ kind: "module", id: scope.id, name: label }),
        },
        { label: t("nav.deleteModule"), onSelect: () => void deleteNodeFlow("module", scope.id, label) },
      ];
    case "submodule":
      return [
        {
          label: t("nav.renameSubmodule"),
          onSelect: () => renameNode({ kind: "submodule", id: scope.id, name: label }),
        },
        {
          label: t("nav.deleteSubmodule"),
          onSelect: () => void deleteNodeFlow("submodule", scope.id, label),
        },
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
  // Clicking the module tree while in "My tasks": switch back to the current project so the table shows that scope.
  if (state.container.kind === "todo" && state.projectId !== null) {
    state.container = { kind: "project", id: state.projectId };
    state.addingSubtaskFor = null;
    subtaskDraft = "";
    // Reset after the container switch: the new default placement comes from this scope.
    resetComposer();
    void reload();
    return;
  }
  // A different scope has a different default placement for the next quick-added task.
  resetComposer();
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
  dom.statusCount.textContent =
    total === 0 ? t("statusbar.noTasks") : tp("statusbar.count", total, { total, done });

  renderComposer();
}

/** Placement implied by the current scope: "All" and "Default" both mean unclassified. */
function scopeTarget(): ScopeTarget {
  if (state.container.kind !== "project") {
    return { moduleId: null, submoduleId: null };
  }
  return resolveTarget(buildGroups(state.tasks, state.modules), state.scope);
}

/** Human-readable placement for the composer trigger and placeholders. */
function targetLabel(target: ScopeTarget): string {
  if (target.submoduleId !== null) {
    for (const module of state.modules) {
      const submodule = module.submodules.find((child) => child.id === target.submoduleId);
      if (submodule) {
        return t("task.modulePath", { module: module.name, submodule: submodule.name });
      }
    }
  }
  if (target.moduleId !== null) {
    const module = state.modules.find((candidate) => candidate.id === target.moduleId);
    if (module) return module.name;
  }
  return t("scope.default");
}

/** Back to "current scope + mid"; used on navigation, database reset and after adding a task. */
function resetComposer(): void {
  composer = { target: scopeTarget(), priority: "mid" };
}

/** A module / submodule chosen earlier may be gone (deleted or the container switched). */
function composerTargetValid(): boolean {
  const { target } = composer;
  if (target.submoduleId !== null) {
    return state.modules.some(
      (module) =>
        module.id === target.moduleId &&
        module.submodules.some((child) => child.id === target.submoduleId),
    );
  }
  if (target.moduleId !== null) {
    return (
      state.container.kind === "project" &&
      state.modules.some((module) => module.id === target.moduleId)
    );
  }
  return true;
}

/**
 * Keep both triggers and the input placeholder in sync with the composer.
 * The placeholder repeats the placement so the user knows where Enter will land.
 */
function renderComposer(): void {
  // Do not keep a dangling module id: fall back to the scope default, leaving priority alone.
  if (!composerTargetValid()) composer.target = scopeTarget();

  const target = targetLabel(composer.target);
  dom.taskModuleLabel.textContent = target;
  const moduleHint = t("task.moduleAria", { target });
  dom.taskModule.title = moduleHint;
  dom.taskModule.setAttribute("aria-label", moduleHint);

  const priority = priorityLabel(composer.priority);
  dom.taskPriorityLabel.textContent = priority;
  const priorityHint = t("task.priorityAria", { priority });
  dom.taskPriority.title = priorityHint;
  dom.taskPriority.setAttribute("aria-label", priorityHint);

  dom.taskInput.placeholder =
    state.container.kind === "project"
      ? t("task.addPlaceholderScoped", { target })
      : t("task.addPlaceholder");
}

function chooseComposerTarget(target: ScopeTarget): void {
  composer.target = target;
  renderComposer();
  dom.taskInput.focus();
}

function chooseComposerPriority(priority: Priority): void {
  composer.priority = priority;
  renderComposer();
  dom.taskInput.focus();
}

/**
 * Module dropdown for the next quick-added task. Flat rather than nested submenus,
 * so a long module list stays inside the viewport near the bottom edge.
 */
function openComposerModuleMenu(): void {
  const items: MenuItem[] = [
    {
      label: t("scope.default"),
      checked: composer.target.moduleId === null && composer.target.submoduleId === null,
      onSelect: () => chooseComposerTarget({ moduleId: null, submoduleId: null }),
    },
  ];

  // "My tasks" has no modules: every task there is unclassified, so only "Default" is offered.
  if (state.container.kind === "project") {
    for (const module of state.modules) {
      items.push({
        label: module.name,
        checked: composer.target.moduleId === module.id && composer.target.submoduleId === null,
        onSelect: () => chooseComposerTarget({ moduleId: module.id, submoduleId: null }),
      });
      for (const submodule of module.submodules) {
        items.push({
          label: t("task.modulePath", { module: module.name, submodule: submodule.name }),
          checked: composer.target.submoduleId === submodule.id,
          onSelect: () =>
            chooseComposerTarget({ moduleId: module.id, submoduleId: submodule.id }),
        });
      }
    }
  }

  openMenu({
    items,
    position: { kind: "anchor", element: dom.taskModule },
    label: t("scope.module"),
    restoreFocus: dom.taskModule,
  });
}

/** Priority dropdown for the next quick-added task. */
function openComposerPriorityMenu(): void {
  openMenu({
    items: PRIORITY_ORDER.map((priority) => ({
      label: priorityLabel(priority),
      checked: composer.priority === priority,
      onSelect: () => chooseComposerPriority(priority),
    })),
    position: { kind: "anchor", element: dom.taskPriority },
    label: t("priority.label"),
    restoreFocus: dom.taskPriority,
  });
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
      t("filter.status"),
      STATUS_ORDER.map((status) => ({
        value: status,
        label: statusLabel(status),
        active: state.statusFilter.has(status),
      })),
    ),
    renderFilterGroup(
      "priority",
      t("filter.priority"),
      PRIORITY_ORDER.map((priority) => ({
        value: priority,
        label: priorityLabel(priority),
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
  group.setAttribute("aria-label", t("filter.groupAria", { label }));
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
  // A full redraw replaces the button; return focus to the chip just toggled so keyboard use is not interrupted.
  dom.filters
    .querySelector<HTMLButtonElement>(
      `button[data-filter-kind="${kind}"][data-filter-value="${value}"]`,
    )
    ?.focus();
}

/** Bucket subtasks by parent; each bucket uses the same ordering as parent tasks in a group (status → priority → creation time). */
function groupChildren(tasks: readonly Task[]): Map<number, Task[]> {
  const map = new Map<number, Task[]>();
  for (const task of tasks) {
    if (task.parentTaskId === null) continue;
    const bucket = map.get(task.parentTaskId);
    if (bucket) bucket.push(task);
    else map.set(task.parentTaskId, [task]);
  }
  for (const [parentId, children] of map) map.set(parentId, sortTasks(children, state.sort));
  return map;
}

function renderTable(groups: Group[]): void {
  const childrenByParent = groupChildren(state.tasks);
  const filters = currentFilters();
  dom.body.replaceChildren();

  // "My tasks" has no modules, so no group rows: the table is flat.
  if (state.container.kind === "todo") {
    const parents = sortTasks(
      state.tasks.filter((task) => task.parentTaskId === null),
      state.sort,
    );
    for (const task of parents) {
      renderTaskTree(task, childrenByParent.get(task.id) ?? [], dom.body, filters);
    }
    const hasMatch = parents.some((task) =>
      taskTreeMatchesFilters(task, childrenByParent.get(task.id) ?? [], filters),
    );
    setEmptyState(
      !hasMatch,
      parents.length === 0 ? t("empty.todo") : t("empty.filtered"),
    );
    focusSubtaskInput();
    return;
  }

  const visible = selectScope(groups, state.scope);
  for (const group of visible) renderGroup(group, dom.body, childrenByParent, filters);

  const { total } = countGroups(visible);
  const where =
    state.scope.kind === "project"
      ? t("empty.thisProject")
      : t("empty.quoted", { label: scopeLabel(groups, state.scope) });
  const hasMatch = visible.some((group) => groupHasMatchingTaskTree(group, childrenByParent, filters));
  setEmptyState(
    !hasMatch,
    total === 0 ? t("empty.scope", { where }) : t("empty.filtered"),
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

/** Depth-first: group row → tasks in the group (including subtasks) → child groups (indented one level). */
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
  caret.title = t(collapsed ? "common.expand" : "common.collapse");
  caret.setAttribute("aria-label", caret.title);
  caret.append(icon("chevron"));
  caret.addEventListener("click", () => {
    if (collapsed) state.collapsedGroups.delete(key);
    else state.collapsedGroups.add(key);
    render();
  });

  label.append(caret, el("span", undefined, group.label ?? t("scope.default")));
  if (group.children.length > 0) {
    label.append(
      el(
        "span",
        "group-meta",
        tp("group.submoduleCount", group.children.length, { count: group.children.length }),
      ),
    );
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
  /** Number of subtasks shown under the current filter; decides whether to draw the collapse caret. */
  childCount: number;
  /** Subtask done counts always use all subtasks and are unaffected by filters. */
  childDone: number;
  childTotal: number;
  isContext?: boolean;
}

/**
 * Parent task + expanded-by-default subtasks + inline add row.
 *
 * When the parent misses but a subtask matches, keep the parent as dimmed context; hide the whole tree only when neither matches.
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
    row.title = t("task.contextHint");
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

  // While editing in the row, do not steal right-click: right-clicks in an input still use the browser default.
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

/** Task context menu: set status / move / demote or promote (one item each for top-level and subtasks) and delete. */
function openTaskMenu(task: Task, event: MouseEvent): void {
  const items: MenuItem[] = [];
  const position: MenuPosition = { kind: "point", x: event.clientX, y: event.clientY };

  // The status cell can only cycle; the context menu lays out all three states directly, with the current one checked.
  items.push({
    label: t("menu.setStatus"),
    children: STATUS_ORDER.map((status) => ({
      label: statusLabel(status),
      checked: status === task.status,
      onSelect: () => void changeStatus(task, status),
    })),
  });

  if (task.parentTaskId === null) {
    items.push({ label: t("menu.moveTo"), onSelect: () => void openMoveMenu(task, position) });
    items.push({
      label: t("menu.demoteToSubtask"),
      onSelect: () => openParentTargetMenu(task, position, "demote"),
    });
  } else {
    items.push({
      label: t("menu.moveToParent"),
      onSelect: () => openParentTargetMenu(task, position, "retarget"),
    });
    items.push({ label: t("menu.promoteToParent"), onSelect: () => void promoteTask(task) });
  }
  // Separate delete from the structural actions above so it is not clicked by mistake in a dense menu.
  items.push({ type: "separator", label: "" });
  items.push({ label: t("menu.deleteTask"), onSelect: () => void deleteTaskFlow(task) });
  const row = event.currentTarget instanceof HTMLElement ? event.currentTarget : null;
  openMenu({
    items,
    position,
    onClose: () => row?.classList.remove("is-menu-open"),
  });
  // Add after openMenu: it first closes any previous menu, whose onClose would
  // otherwise remove the class we just set on the same row.
  row?.classList.add("is-menu-open");
}

type ParentMoveMode = "demote" | "retarget";

/** Candidate parents are top-level tasks in this container, excluding the task itself,
 * its current parent and completed tasks; both parents and leaf tasks are allowed. */
function isEligibleParent(task: Task, candidate: Task): boolean {
  return (
    candidate.parentTaskId === null &&
    candidate.id !== task.id &&
    candidate.id !== task.parentTaskId &&
    candidate.status !== "done"
  );
}

/** Target parents are shown grouped like the current container; subtasks cannot become parents. */
function parentTargetItem(task: Task, target: Task, mode: ParentMoveMode): MenuItem {
  return { label: target.title, onSelect: () => void moveTaskUnderParent(task, target, mode) };
}

function parentGroupItem(task: Task, group: Group, mode: ParentMoveMode): MenuItem | null {
  const tasks = sortParentCandidates(
    group.tasks.filter((candidate) => isEligibleParent(task, candidate)),
  );
  const children = group.children
    .map((child) => parentGroupItem(task, child, mode))
    .filter((child): child is MenuItem => child !== null);

  if (tasks.length === 0 && children.length === 0) return null;
  return {
    label: group.label ?? t("scope.default"),
    children: [...tasks.map((target) => parentTargetItem(task, target, mode)), ...children],
  };
}

/** Target-parent menu for demoting a top-level task or moving a subtask to another parent. */
function openParentTargetMenu(
  task: Task,
  position: MenuPosition,
  mode: ParentMoveMode,
): void {
  if (mode === "demote") {
    const children = state.tasks.filter((candidate) => candidate.parentTaskId === task.id);
    if (children.length > 0) {
      showNotice(
        tp("menu.demoteHasChildren", children.length, {
          title: task.title,
          count: children.length,
        }),
      );
      return;
    }
  }

  const targets = state.tasks.filter((candidate) => isEligibleParent(task, candidate));
  if (targets.length === 0) {
    showNotice(t(mode === "retarget" ? "menu.noOtherParents" : "menu.noParentTargets"));
    return;
  }

  const items =
    state.container.kind === "todo"
      ? sortParentCandidates(targets).map((target) => parentTargetItem(task, target, mode))
      : buildGroups(state.tasks, state.modules)
          .map((group) => parentGroupItem(task, group, mode))
          .filter((item): item is MenuItem => item !== null);

  openMenu({
    items,
    position,
    label: t(mode === "demote" ? "menu.demoteLabel" : "menu.moveToParentLabel"),
  });
}

/** Redraw only on a successful move; expand the target so the new subtask is visible. */
async function moveTaskUnderParent(
  task: Task,
  target: Task,
  mode: ParentMoveMode,
): Promise<void> {
  const clamped = PRIORITY_ORDER.indexOf(task.priority) < PRIORITY_ORDER.indexOf(target.priority);
  try {
    const outcome = await api.reparentTask(task.id, target.id);
    state.collapsedTasks.delete(target.id);
    await reload();
    flashTaskTree(target.id);
    const priority = { priority: priorityLabel(target.priority) };
    showNotice(
      clamped
        ? t(
            mode === "demote"
              ? "toast.demotedWithPriority"
              : "toast.movedUnderParentWithPriority",
            { title: task.title, target: target.title, ...priority },
          )
        : t(mode === "demote" ? "toast.demoted" : "toast.movedUnderParent", {
            title: task.title,
            target: target.title,
          }),
    );
    for (const notice of outcome.notices) showNotice(notice);
    await confirmParentProposals(outcome.parentProposals);
  } catch (error) {
    showNotice(describe(error), "error");
  }
}

/** Redraw only on successful promotion; if the former parent was changed by aggregation, the backend returns the notice. */
async function promoteTask(task: Task): Promise<void> {
  try {
    const outcome = await api.reparentTask(task.id, null);
    await reload();
    flashTaskTree(task.id);
    showNotice(t("toast.promoted", { title: task.title }));
    for (const notice of outcome.notices) showNotice(notice);
    await confirmParentProposals(outcome.parentProposals);
  } catch (error) {
    showNotice(describe(error), "error");
  }
}

/**
 * One column each for created / start / end, read-only. Creation time is written when a task is created; start and end
 * are maintained automatically with status changes. The table has no input entry for them.
 */
function renderTimeCell(task: Task, field: TimeField): HTMLTableCellElement {
  const cell = el("td", "cell-time");
  const value =
    field === "created" ? task.createdAt : field === "start" ? task.startAt : task.endAt;
  const label = t(TIME_LABEL_KEYS[field]);
  const span = el("span", value === null ? "time is-empty" : "time", formatSchedule(value));
  span.title = t("time.tooltip", {
    label,
    value: value === null ? t("time.notRecorded") : formatSchedule(value),
    hint: t(TIME_HINT_KEYS[field]),
  });
  span.setAttribute("aria-label", span.title);
  cell.append(span);
  return cell;
}

function renderStatusCell(task: Task): HTMLTableCellElement {
  const cell = el("td", "cell-status");
  const button = el("button", `status status-${task.status}`);
  button.type = "button";
  button.title = t("status.tooltip", {
    current: statusLabel(task.status),
    next: statusLabel(NEXT_STATUS[task.status]),
  });
  button.setAttribute("aria-label", t("status.aria", { status: statusLabel(task.status) }));
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
  const button = el("button", `priority priority-${task.priority}`, priorityLabel(task.priority));
  button.type = "button";
  const level = priorityLabel(task.priority);
  button.title =
    task.parentTaskId === null
      ? t("priority.tooltip", { level })
      : t("priority.tooltipChild", { level });
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
  title.title = t("task.renameHint");
  title.addEventListener("dblclick", () => beginRename(title, task));

  // Subtasks are the end of the line: no caret, no "+", hierarchy shown by indentation alone.
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
    caret.title = t(collapsed ? "task.expandSubtasks" : "task.collapseSubtasks");
    caret.setAttribute("aria-label", caret.title);
    caret.setAttribute("aria-expanded", String(!collapsed));
    caret.append(icon("chevron"));
    caret.addEventListener("click", () => toggleTaskCollapsed(task.id));
    wrap.append(caret);
  } else {
    // Reserve caret width even without subtasks, so the title does not shift when subtasks appear.
    wrap.append(el("span", "task-caret task-caret-empty"));
  }

  wrap.append(title);

  if (options.childTotal > 0) {
    const count = el("span", "task-count", `${options.childDone}/${options.childTotal}`);
    count.title = tp("task.subtaskDoneCount", options.childTotal, {
      done: options.childDone,
      total: options.childTotal,
    });
    wrap.append(count);
  }

  const plus = el("button", "subtask-add");
  plus.type = "button";
  plus.title = t("task.addSubtask");
  plus.setAttribute("aria-label", t("task.addSubtaskFor", { title: task.title }));
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
  // Whether or not input is already active, invalidate the previous input's blur callback.
  subtaskInputSession += 1;
  state.collapsedTasks.delete(parentId); // the input row follows the subtasks, so expand first

  if (state.addingSubtaskFor === parentId) {
    focusSubtaskInput();
    return;
  }

  state.addingSubtaskFor = parentId;
  subtaskDraft = "";
  render();
}

/** Inline subtask input under a parent row: Enter keeps adding, Esc cancels, blur saves non-empty content. */
function renderSubtaskAddRow(parent: Task): HTMLTableRowElement {
  const session = subtaskInputSession;
  const row = el("tr", "row-subtask-add");
  row.append(el("td"));
  const cell = el("td", "cell-title");

  const input = el("input", "subtask-input");
  input.value = subtaskDraft;
  input.placeholder = t("task.subtaskPlaceholder");
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

    // Redraw to clear the input first, then send the command; on failure the draft is restored.
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
  // Blur saves non-empty content, for the same reason as in-place rename. Defer one tick: when clicking another "+",
  // blur fires before click, and a synchronous redraw would replace the button and swallow the following click.
  input.addEventListener("blur", () => {
    window.setTimeout(() => {
      // `isConnected` distinguishes "the user clicked elsewhere" from "a redraw replaced this input":
      // the latter also fires blur, but the input is no longer in the document and must not count as a submit.
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
    // On failure put the draft back; do not lose what was typed because of one backend rejection.
    state.addingSubtaskFor = parent.id;
    subtaskDraft = title;
    render();
    showNotice(describe(error), "error");
  }
}

function renderNoteCell(task: Task): HTMLTableCellElement {
  const cell = el("td", "cell-note");
  // Summary: newlines and runs of whitespace collapse to single spaces, so the DOM always holds one line.
  const summary = task.note.trim().replace(/\s+/g, " ");
  const button = el("button", summary ? "note" : "note is-empty", summary || t("common.dash"));
  button.type = "button";
  button.title = t("note.edit");
  button.setAttribute("aria-label", t("note.aria", { summary: summary || t("note.empty") }));
  button.addEventListener("click", () => void editNote(task));
  cell.append(button);
  return cell;
}

// ---------------------------------------------------------------------------
// Write operations (in-cell editing)
// ---------------------------------------------------------------------------

/** Note: edited as-is in a dialog; an empty string clears it; no change means no command. */
async function editNote(task: Task): Promise<void> {
  const note = await askNote({
    title: t("note.title"),
    label: t("note.dialogLabel", { title: task.title }),
    value: task.note,
    confirmText: t("common.save"),
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

/** The current target cannot be chosen again, so it simply does not appear in the menu. */
function moveTargetItem(task: Task, label: string, target: MoveTarget): MenuItem | null {
  return isSameTarget(task, target)
    ? null
    : { label, onSelect: () => void moveTaskTo(task, target) };
}

/**
 * Move targets fold into a "project → module → submodule" tree: by default only My tasks and projects show;
 * expanding a project reveals Default and its modules; expanding a module reveals its submodules.
 * Short labels in the menu express ownership through hierarchy, instead of repeating the full `project › module › submodule`.
 */
async function moveMenuItems(task: Task): Promise<MenuItem[]> {
  const items: MenuItem[] = [];
  const todo = moveTargetItem(task, t("nav.todo"), {
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

    const unclassified = moveTargetItem(task, t("scope.default"), {
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

      const wholeModule = moveTargetItem(task, t("menu.moveWholeModule"), moduleTarget);
      if (submoduleItems.length === 0) {
        // No submodules to expand: make the module a leaf item, saving one click.
        if (wholeModule) children.push({ ...wholeModule, label: module.name });
        continue;
      }
      children.push({
        label: module.name,
        children: wholeModule ? [wholeModule, ...submoduleItems] : submoduleItems,
      });
    }

    if (children.length > 0) items.push({ label: projectName(project), children });
  });

  return items;
}

/**
 * Second-level "Move to…" menu: still uses the same `openMenu`, reopened at the right-click position.
 * Project → module → submodule are opened as side panels by the menu component; the parent panel never reflows.
 * The current location does not appear in the list.
 */
async function openMoveMenu(task: Task, position: MenuPosition): Promise<void> {
  try {
    const items = await moveMenuItems(task);
    if (items.length === 0) {
      showNotice(t("menu.noMoveTargets"));
      return;
    }

    openMenu({ items, position, label: t("menu.moveLabel") });
  } catch (error) {
    showNotice(describe(error), "error");
  }
}

/** Redraw and notify only on successful move; leave things as they are on failure, never make a row vanish. */
async function moveTaskTo(task: Task, target: MoveTarget): Promise<void> {
  try {
    await api.moveTask(task.id, target);
    await reload();
    flashTaskTree(task.id);
    showNotice(t("toast.moved", { title: task.title }));
  } catch (error) {
    showNotice(describe(error), "error");
  }
}

/** Status cell click: cycle to the next of the three states. */
async function cycleStatus(task: Task): Promise<void> {
  await changeStatus(task, NEXT_STATUS[task.status]);
}

/**
 * Set a status (shared by status-cell cycling and the right-click "Set status…" menu): plan first asks "is confirmation needed";
 * show the dialog only when needs_confirm is true, then apply. If the target equals the current status, nothing is written.
 */
async function changeStatus(task: Task, next: Status): Promise<void> {
  if (next === task.status) return;
  try {
    const plan = await api.planSetStatus(task.id, next);
    let cascade = false;

    if (plan.needsConfirm) {
      const confirmed = await askConfirm({
        title: t("confirm.markAs", { status: statusLabel(next) }),
        body: plan.reason ? translateBackendMessage(plan.reason) : t("error.unknown"),
        confirmText: t("common.confirm"),
      });
      if (!confirmed) return;
      cascade = true;
    }

    const outcome = await api.applySetStatus(task.id, next, cascade);
    await reload();
    // When "Done" is not checked, completing a task removes it from the view immediately; say where it went so the action does not look ineffective.
    const stillVisible = document.querySelector(`tr[data-task-id="${task.id}"]`) !== null;
    if (next === "done" && !stillVisible) {
      showNotice(t("task.completedHidden"));
    }
    for (const notice of outcome.notices) showNotice(notice);
    await confirmParentProposals(outcome.parentProposals);
  } catch (error) {
    showNotice(describe(error), "error");
  }
}

/** Priority popover. After the change the row jumps by in-group order, so give the new position a brief highlight. */
function openPriorityMenu(anchor: HTMLButtonElement, task: Task): void {
  // A subtask's choices are capped by its parent's level: higher options simply do not appear.
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
      label: priorityLabel(priority),
      checked: priority === task.priority,
      onSelect: () => {
        if (priority !== task.priority) void applyPriority(task, priority);
      },
    })),
    position: { kind: "anchor", element: anchor },
    label: t("priority.label"),
    restoreFocus: anchor,
  });
}

async function applyPriority(task: Task, priority: Priority): Promise<void> {
  // Lowering below some subtasks makes the backend lower them too; count them first, then show a notice afterward.
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
    if (cascaded > 0) {
      showNotice(tp("toast.priorityCascaded", cascaded, { count: cascaded }));
    }
    // Changing a parent's level reorders the whole tree, so flash one second longer to help track the new position.
    flashTaskTree(task.id, task.parentTaskId === null);
  } catch (error) {
    showNotice(describe(error), "error");
  }
}

/** In-place rename: replace only the title itself; the parent's caret, count and "+" stay put. */
function beginRename(title: HTMLSpanElement, task: Task): void {
  const cell = title.closest<HTMLTableCellElement>("td");
  if (!cell || cell.querySelector(".title-input")) return;

  // Use textarea rather than input: when the title exceeds one line, the editor must wrap and grow with it,
  // matching the read-only layout; Enter still submits and does not insert a newline.
  const input = el("textarea", "title-input");
  input.rows = 1;
  input.value = task.title;
  input.spellcheck = false;
  input.autocomplete = "off";
  // A parent's title sits in a flex row: the input spans from the title's position to the cell's right edge, as count and "+" step aside.
  const row = title.parentElement;
  if (row?.classList.contains("task-title-row")) {
    row.classList.add("is-renaming");
  }

  // Height follows content: reset to zero, read scrollHeight, and grow one line per extra line.
  const autosize = (): void => {
    input.style.height = "0px";
    input.style.height = `${input.scrollHeight + 2}px`;
  };
  input.addEventListener("input", autosize);

  title.replaceWith(input);
  input.focus();
  input.select();
  autosize();

  // Enter and blur may both fire; accept only the first result.
  let settled = false;
  const finish = (commit: boolean): void => {
    if (settled) return;
    settled = true;

    // Fold pasted newlines into spaces: titles wrap visually only; newline characters are never written to the DB.
    const next = input.value.replace(/\s*\n+\s*/g, " ").trim();
    // Empty or unchanged: restore the original without writing (the backend would reject it too, so no need for the round trip).
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

  // Clicking elsewhere also counts as done: moving focus in the table is the usual "I'm done" action,
  // and losing what was just typed is worse than saving once too often. Esc remains the only "never mind".
  input.addEventListener("blur", () => finish(true));
}

async function commitRename(task: Task, title: string): Promise<void> {
  try {
    await api.renameTask(task.id, title);
    await reload();
  } catch (error) {
    showNotice(describe(error), "error");
    render(); // retract the input even on failure; never stop halfway
  }
}

/**
 * After a priority change / move, flash the moved parent and its accompanying subtask rows together.
 *
 * `long` is for a parent priority change: the whole tree, subtasks included, moves at once; 1 second is too quick,
 * so a static highlight for 2 seconds is easier to follow.
 */
function flashTaskTree(taskId: number, long = false): void {
  const ids = [
    taskId,
    ...state.tasks.filter((task) => task.parentTaskId === taskId).map((task) => task.id),
  ];
  for (const id of ids) {
    const row = dom.body.querySelector<HTMLTableRowElement>(`tr[data-task-id="${id}"]`);
    if (!row) continue;
    row.classList.add("just-moved");
    if (long) {
      row.classList.add("just-moved-long");
      // A static highlight does not fade on its own, so remove the class on a timer; a no-op if the row was already redrawn.
      window.setTimeout(() => row.classList.remove("just-moved-long"), 2000);
    }
  }
}

// ---------------------------------------------------------------------------
// Confirm / text-input dialogs (one shared modal)
// ---------------------------------------------------------------------------

/** Wipe the previous shape before every dialog: confirm and text-input share this one modal. */
function resetModal(): void {
  dom.modalBody.hidden = false;
  dom.modalBody.textContent = "";
  dom.modalField.hidden = true;
  dom.modalInput.hidden = false;
  dom.modalInput.value = "";
  dom.modalTextarea.hidden = true;
  dom.modalTextarea.value = "";
  dom.modalCancel.hidden = false;
  dom.modalCard.classList.remove("is-wide", "is-alert");
}

/**
 * The "cancel" entry of the currently open dialog.
 *
 * One modal allows only one owner: a new dialog first dismisses the old one as canceled,
 * otherwise two `await`s hang at once and fight over the same DOM listeners, running two commands per Enter.
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

/** The common "tell the impact, then confirm" shape. Returns a Promise the caller can await directly. */
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

interface AlertOptions {
  title: string;
  body: string;
  confirmText: string;
}

/** A warning dialog with a single confirm button (no cancel path). */
function showAlert(options: AlertOptions): Promise<void> {
  return new Promise((resolve) => {
    let closed = false;
    beginDialog();
    resetModal();
    dom.modalTitle.textContent = options.title;
    dom.modalBody.textContent = options.body;
    dom.modalOk.textContent = options.confirmText;
    dom.modalCancel.hidden = true;
    dom.modalCard.classList.add("is-alert");
    dom.modal.hidden = false;
    dom.modalOk.focus();

    const close = (): void => {
      if (closed) return;
      closed = true;
      endDialog();
      dom.modal.hidden = true;
      dom.modalCancel.hidden = false;
      dom.modalCard.classList.remove("is-alert");
      dom.modalOk.removeEventListener("click", close);
      dom.modal.removeEventListener("click", onBackdrop);
      document.removeEventListener("keydown", onKey);
      resolve();
    };
    const onBackdrop = (event: MouseEvent): void => {
      if (event.target === dom.modal) close();
    };
    const onKey = (event: KeyboardEvent): void => {
      if (event.key === "Escape" || event.key === "Enter") close();
    };

    dom.modalOk.addEventListener("click", close);
    dom.modal.addEventListener("click", onBackdrop);
    document.addEventListener("keydown", onKey);

    dismissActiveDialog = close;
  });
}

/** Duplicate module names are blocking: show a warning dialog instead of a toast. */
async function handleDuplicateModuleName(error: unknown): Promise<boolean> {
  if (backendMessageKey(error) !== "validation.moduleNameDuplicate") return false;
  await showAlert({
    title: t("dialog.duplicateModuleName"),
    body: describe(error),
    confirmText: t("common.ok"),
  });
  return true;
}

/**
 * Aggregation never writes a parent as done on its own: it proposes the change, and the
 * user decides here. Declining leaves the parent as it is. A retarget may propose two
 * parents (the one left and the one joined), so ask for each one in turn.
 */
async function confirmParentProposals(
  proposals: ParentStatusProposal[],
): Promise<void> {
  if (proposals.length === 0) return;

  const accepted: ParentStatusProposal[] = [];
  for (const proposal of proposals) {
    const confirmed = await askConfirm({
      title: t("confirm.parentStatusTitle"),
      body: translateBackendMessage(proposal.message),
      confirmText: t("common.confirm"),
    });
    if (confirmed) accepted.push(proposal);
  }
  if (accepted.length === 0) return;

  for (const proposal of accepted) {
    try {
      await api.applySetStatus(proposal.parentTaskId, proposal.status, false);
    } catch (error) {
      showNotice(describe(error), "error");
    }
  }
  await reload();
}

interface TextOptions {
  title: string;
  /** Small text above the input, usually "old name → new name" */
  label: string;
  value: string;
  confirmText: string;
}

/** Single text-input dialog. Cancel returns `null`; confirm returns a trimmed non-empty string. */
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

    // A blank name neither closes the dialog nor writes to the DB: closing would imply an empty name was saved
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
 * Multiline note dialog. Key differences from `askText`:
 * - Enter inserts a newline, not a submit; confirm happens only via the button.
 * - The return value is **not trimmed**; an empty string clears the note.
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
    // Put the caret at the end: the dialog is opened to keep writing.
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
// Events
// ---------------------------------------------------------------------------

dom.taskForm.addEventListener("submit", (event) => {
  event.preventDefault();
  void createTask();
});

async function createProjectFlow(): Promise<void> {
  const name = await askText({
    title: t("dialog.newProject"),
    label: t("dialog.projectName"),
    value: "",
    confirmText: t("common.create"),
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
    title: t("dialog.newModule"),
    label: t("dialog.moduleName"),
    value: "",
    confirmText: t("common.create"),
  });
  if (name === null) return;

  try {
    await api.createModule(projectId, name);
    // Created in another project: switch the view there, or the new thing will not be visible
    const project = state.projects.find((p) => p.id === projectId);
    if (project && !isCurrent(project)) {
      await selectProject(project);
      return;
    }
    await reload();
  } catch (error) {
    if (await handleDuplicateModuleName(error)) return;
    showNotice(describe(error), "error");
  }
}

async function createSubmoduleIn(moduleId: number): Promise<void> {
  const name = await askText({
    title: t("dialog.newSubmodule"),
    label: t("dialog.submoduleName"),
    value: "",
    confirmText: t("common.create"),
  });
  if (name === null) return;

  try {
    await api.createSubmodule(moduleId, name);
    // Not seeing it right after adding is as good as not adding: expand this level too
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

/**
 * Structure renaming uses in-place editing: the name itself becomes an input, with no dialog and no leaving the sidebar.
 * Double-clicking a name and the context menu's "Rename…" both land here.
 */
function renameNode(target: RenameTarget): void {
  if (target.kind === "project") {
    beginProjectRename(target.id, target.name);
    return;
  }
  const button = dom.scopeSection.querySelector<HTMLButtonElement>(
    `.scope-name[data-kind="${target.kind}"][data-id="${target.id}"]`,
  );
  if (button) beginScopeRename(button, target.kind, target.id, target.name);
}

function beginProjectRename(id: number, currentName: string): void {
  if (dom.projectHeading.querySelector(".side-input")) return;
  const input = el("input", "side-input heading-input");
  input.value = currentName;
  input.spellcheck = false;
  input.autocomplete = "off";
  dom.projectHeading.replaceChildren(input);
  beginInlineEdit(input, currentName, (name) => api.renameProject(id, name));
}

function beginScopeRename(
  button: HTMLButtonElement,
  kind: "module" | "submodule",
  id: number,
  currentName: string,
): void {
  if (!button.isConnected) return;
  const input = el("input", "side-input scope-input");
  input.value = currentName;
  input.spellcheck = false;
  input.autocomplete = "off";
  button.replaceWith(input);
  beginInlineEdit(input, currentName, (name) =>
    kind === "module" ? api.renameModule(id, name) : api.renameSubmodule(id, name),
  );
}

/** Shared finish for in-place editing: Enter / blur saves, Esc cancels; empty or unchanged just redraws the original. */
function beginInlineEdit(
  input: HTMLInputElement,
  original: string,
  commit: (name: string) => Promise<unknown>,
): void {
  let settled = false;
  const finish = (save: boolean): void => {
    if (settled) return;
    settled = true;

    const next = input.value.trim();
    if (!save || next === "" || next === original) {
      render();
      return;
    }

    void (async () => {
      try {
        await commit(next);
        await reload();
      } catch (error) {
        if (await handleDuplicateModuleName(error)) {
          render();
          return;
        }
        showNotice(describe(error), "error");
        render(); // retract the input even on failure; never stop half-edited
      }
    })();
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
  // Clicking elsewhere is equivalent to saving, consistent with task-title in-place rename.
  input.addEventListener("blur", () => finish(true));

  input.focus();
  input.select();
}

/** Delete task: first ask the backend how many subtasks go with it, then confirm. */
async function deleteTaskFlow(task: Task): Promise<void> {
  try {
    const plan = await api.planDeleteTask(task.id);
    const extra = plan.taskCount - 1;
    const body =
      extra > 0
        ? tp("confirm.deleteTaskWithChildren", extra, { count: extra })
        : t("confirm.deleteTaskAlone");

    const confirmed = await askConfirm({
      title: t("confirm.deleteTask"),
      body,
      confirmText: t("common.delete"),
    });
    if (!confirmed) return;

    const outcome = await api.deleteTask(task.id);
    state.collapsedTasks.delete(task.id);
    if (state.addingSubtaskFor === task.id) {
      state.addingSubtaskFor = null;
      subtaskDraft = "";
    }
    await reload();
    showNotice(t("toast.taskDeleted"));
    for (const notice of outcome.notices) showNotice(notice);
    await confirmParentProposals(outcome.parentProposals);
  } catch (error) {
    showNotice(describe(error), "error");
  }
}

/** Delete module / submodule: tasks inside are deleted too, and the count includes tasks in submodules. */
async function deleteNodeFlow(kind: NodeKind, id: number, label: string): Promise<void> {
  const noun = kind === "module" ? t("scope.module") : t("scope.submodule");
  try {
    const plan = await api.planDeleteNode(kind, id);
    const body =
      plan.taskCount > 0
        ? tp("confirm.deleteNodeWithTasks", plan.taskCount, {
            noun,
            label,
            count: plan.taskCount,
          })
        : t("confirm.deleteNode", { noun, label });

    const confirmed = await askConfirm({
      title: t(kind === "module" ? "confirm.deleteModule" : "confirm.deleteSubmodule"),
      body,
      confirmText: t("common.delete"),
    });
    if (!confirmed) return;

    // If the scope sits inside the deleted subtree, pull it back to "All" — never stay somewhere that no longer exists.
    const scopeWasInside = scopeInsideNode(kind, id);
    const submoduleIds =
      kind === "module" ? (state.modules.find((m) => m.id === id)?.submodules ?? []) : [];
    await api.deleteNode(kind, id);
    state.collapsedNodes.delete(id);
    for (const submodule of submoduleIds) state.collapsedNodes.delete(submodule.id);
    if (scopeWasInside) {
      state.scope = { kind: "project" };
      resetComposer();
    }

    await reload();
    showNotice(t("toast.nodeDeleted", { noun, label }));
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

/** Delete project: tasks inside are deleted too; if currently in this project, fall back to "My tasks". */
async function deleteProjectFlow(project: Project): Promise<void> {
  if (project.id === null) return;
  const id = project.id;

  try {
    const plan = await api.planDeleteProject(id);
    const name = projectName(project);
    const body =
      plan.taskCount > 0
        ? tp("confirm.deleteProjectWithTasks", plan.taskCount, {
            name,
            count: plan.taskCount,
          })
        : t("confirm.deleteProjectOnly", { name });

    const confirmed = await askConfirm({
      title: t("confirm.deleteProject"),
      body,
      confirmText: t("common.delete"),
    });
    if (!confirmed) return;

    const wasHere = state.container.kind === "project" && state.container.id === id;
    const wasCurrentProject = state.projectId === id;
    await api.deleteProject(id);

    if (wasHere || wasCurrentProject) {
      if (wasHere) state.container = { kind: "todo" };
      if (wasCurrentProject) state.projectId = null;
      state.scope = { kind: "project" };
      resetComposer();
      state.collapsedNodes.clear();
      state.collapsedGroups.clear();
      state.collapsedTasks.clear();
      state.addingSubtaskFor = null;
      subtaskDraft = "";
      try {
        localStorage.removeItem(PROJECT_STORAGE_KEY);
      } catch {
        // Ignore: unavailable storage does not affect the deletion itself.
      }
    }

    await reload();
    showNotice(t("toast.projectDeleted", { name: projectName(project) }));
  } catch (error) {
    showNotice(describe(error), "error");
  }
}

/** Dev builds only: delete the live DB and rebuild. Release builds have no button and register no command. */
async function resetDatabaseFlow(): Promise<void> {
  const confirmed = await askConfirm({
    title: t("confirm.resetDatabase"),
    body: t("confirm.resetDatabaseBody"),
    confirmText: t("confirm.deleteAndRebuild"),
  });
  if (!confirmed) return;

  try {
    await api.resetDatabase();
    // The database has been replaced: return the view to "My tasks"; old group/collapse keys are all invalid.
    state.container = { kind: "todo" };
    state.projectId = null;
    state.scope = { kind: "project" };
    resetComposer();
    state.collapsedNodes.clear();
    state.collapsedGroups.clear();
    state.collapsedTasks.clear();
    state.addingSubtaskFor = null;
    subtaskDraft = "";
    try {
      localStorage.removeItem(PROJECT_STORAGE_KEY);
    } catch {
      // Ignore: resetting the database does not depend on clearing local preferences.
    }
    closeMenu();
    await reload();
    showNotice(t("toast.databaseReset"));
  } catch (error) {
    showNotice(describe(error), "error");
  }
}

async function createTask(): Promise<void> {
  const title = dom.taskInput.value.trim();
  if (!title) return;
  const placedIn = targetLabel(composer.target);
  try {
    // The composer starts at the current scope and lets the user override module / priority for this task.
    const created = await api.createTask(
      state.container,
      composer.target,
      title,
      null,
      composer.priority,
    );
    dom.taskInput.value = "";
    resetComposer();
    await reload();
    // A task placed outside the current scope (or hidden by filters) leaves no visible row; say so instead of failing silently.
    const visible = document.querySelector(`tr[data-task-id="${created.task.id}"]`) !== null;
    if (!visible) showNotice(t("toast.taskAddedElsewhere", { target: placedIn }));
    for (const notice of created.notices) showNotice(notice);
  } catch (error) {
    showNotice(describe(error), "error");
  }
}

// ---------------------------------------------------------------------------
// Toasts
// ---------------------------------------------------------------------------

interface NoticeItem {
  message: string;
  kind: "info" | "error";
}

const noticeQueue: NoticeItem[] = [];
let noticeActive = false;
let noticeTimer: number | undefined;

/**
 * A self-dismissing toast at the bottom that blocks nothing.
 *
 * Multiple notices queue FIFO and show one at a time — an auto cascade may return several at once,
 * and overwriting them directly would make the earlier one invisible forever.
 */
function showNotice(message: string | BackendMessage, kind: "info" | "error" = "info"): void {
  noticeQueue.push({
    message: typeof message === "string" ? message : translateBackendMessage(message),
    kind,
  });
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
    3000,
  );
}

// ---------------------------------------------------------------------------
// Startup
// ---------------------------------------------------------------------------

// Module scripts usually run before DOMContentLoaded, but if imported through an entry with await,
// the event may already have passed. Boot must work in both cases.
if (document.readyState === "loading") {
  window.addEventListener("DOMContentLoaded", boot);
} else {
  boot();
}

/**
 * Standalone page when the database cannot open: show only the backend's reason and backup path, touching no commands.
 * The backend points the main window at `index.html?startupError=...` during setup; take over directly here.
 */
interface StartupFailureView {
  error?: BackendMessage;
  detail?: string;
  backupPath?: string | null;
}

/** The backend sends JSON; fall back to raw text for hand-written URLs. */
function parseStartupFailure(raw: string): StartupFailureView {
  try {
    const parsed = JSON.parse(raw) as { error?: unknown; backupPath?: unknown };
    if (parsed && typeof parsed === "object" && typeof parsed.error === "object") {
      const error = parsed.error as BackendMessage;
      if (typeof error.key === "string") {
        return {
          error,
          backupPath: typeof parsed.backupPath === "string" ? parsed.backupPath : null,
        };
      }
    }
  } catch {
    // Not JSON: show the text as-is.
  }
  return { detail: raw };
}

function renderStartupError(raw: string): void {
  document.title = t("startup.documentTitle");
  document.body.classList.add("startup-error-body");

  const failure = parseStartupFailure(raw);
  const detail = failure.error ? translateBackendMessage(failure.error) : (failure.detail ?? raw);
  const card = el("section", "startup-error-card");
  card.append(
    el("h1", "startup-error-title", t("startup.title")),
    el("p", "startup-error-lead", t("startup.lead")),
    el("pre", "startup-error-detail", detail),
  );
  if (failure.backupPath) {
    card.append(
      el("p", "startup-error-hint", t("startup.backup", { path: failure.backupPath })),
    );
  }
  card.append(el("p", "startup-error-hint", t("startup.hint")));

  const panel = el("main", "startup-error");
  panel.append(card);
  document.body.replaceChildren(panel);
}

function boot(): void {
  initLanguage();
  const startupError = new URLSearchParams(location.search).get(STARTUP_ERROR_PARAM);
  if (startupError) {
    renderStartupError(startupError);
    return;
  }
  applyStaticTranslations();
  // The native context menu is useless in the app; project and scope nodes each preventDefault
  // and open their own menus, so this globally disables the browser menu everywhere else.
  document.addEventListener("contextmenu", (event) => {
    event.preventDefault();
  });
  initTheme();
  void initWindowControls();
  // Tauri's drag logic intercepts the drag region first, so the menu's global mousedown never fires; close it in the capture phase instead.
  dom.titlebarDrag.addEventListener("mousedown", () => closeMenu(), true);
  dom.projectBtn.addEventListener("click", openProjectMenu);
  dom.taskModule.addEventListener("click", openComposerModuleMenu);
  dom.taskPriority.addEventListener("click", openComposerPriorityMenu);
  dom.aboutBtn.addEventListener("click", openAboutDialog);
  initTooltips();
  initProjectHeading();
  initSidebarContextMenu();
  initSidebarResizing();
  initColumnResizing();
  initColumnSorting();
  render();
  void reload();
}

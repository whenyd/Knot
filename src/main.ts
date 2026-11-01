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
  type Project,
  type Task,
} from "./api";
import { buildGroups, countGroup, sortTasks, type Group } from "./grouping";

interface State {
  projects: Project[];
  container: Container;
  modules: Module[];
  tasks: Task[];
}

const state: State = {
  projects: [],
  container: { kind: "todo" },
  modules: [],
  tasks: [],
};

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

const dom = {
  todoSection: must<HTMLElement>("todo-section"),
  projectSection: must<HTMLElement>("project-section"),
  projectForm: must<HTMLFormElement>("project-form"),
  projectInput: must<HTMLInputElement>("project-input"),
  projectAdd: must<HTMLButtonElement>("project-add"),
  moduleAdd: must<HTMLButtonElement>("module-add"),
  moduleForm: must<HTMLFormElement>("module-form"),
  moduleInput: must<HTMLInputElement>("module-input"),
  viewTitle: must<HTMLElement>("view-title"),
  viewCount: must<HTMLElement>("view-count"),
  body: must<HTMLTableSectionElement>("grid-body"),
  empty: must<HTMLParagraphElement>("empty"),
  taskForm: must<HTMLFormElement>("task-form"),
  taskInput: must<HTMLInputElement>("task-input"),
  notice: must<HTMLElement>("notice"),
};

// ---------------------------------------------------------------------------
// 加载
// ---------------------------------------------------------------------------

/** 取回当前容器的全部数据并重画。所有写操作之后都调它。 */
async function reload(): Promise<void> {
  try {
    state.projects = await api.listProjects();
    // 取成局部量：容器在 await 之间可能被用户切走，一次重画只认一个容器。
    const container = state.container;
    state.modules = container.kind === "project" ? await api.listModules(container.id) : [];
    state.tasks = await api.listTasks(container);
    render();
  } catch (error) {
    showNotice(describe(error), "error");
  }
}

async function selectProject(project: Project): Promise<void> {
  state.container = project.isTodo || project.id === null
    ? { kind: "todo" }
    : { kind: "project", id: project.id };
  closeModuleForm(); // 切容器时收起就地输入，别把一半输入带到另一个视图
  await reload();
}

// ---------------------------------------------------------------------------
// 渲染
// ---------------------------------------------------------------------------

function render(): void {
  renderSidebar();
  renderToolbar();
  renderTable();
}

function renderSidebar(): void {
  dom.todoSection.replaceChildren();
  dom.projectSection.replaceChildren();

  for (const project of state.projects) {
    const button = renderSideItem(project);
    (project.isTodo ? dom.todoSection : dom.projectSection).append(button);
  }
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
  return button;
}

function isCurrent(project: Project): boolean {
  if (project.isTodo) return state.container.kind === "todo";
  return state.container.kind === "project" && state.container.id === project.id;
}

function currentProject(): Project | undefined {
  const { container } = state;
  if (container.kind === "todo") return state.projects.find((p) => p.isTodo);
  return state.projects.find((p) => p.id === container.id);
}

function renderToolbar(): void {
  dom.viewTitle.textContent = currentProject()?.name ?? "待办";
  dom.moduleAdd.hidden = state.container.kind !== "project" || !dom.moduleForm.hidden;

  const parents = state.tasks.filter((task) => task.parentTaskId === null);
  const done = parents.filter((task) => task.status === "done").length;
  dom.viewCount.textContent = parents.length === 0 ? "暂无任务" : `${done}/${parents.length}`;
}

function renderTable(): void {
  dom.body.replaceChildren();

  const isEmpty = state.tasks.length === 0;
  dom.empty.hidden = !isEmpty;
  dom.empty.textContent =
    state.container.kind === "todo"
      ? "「待办」里还没有任务。在下面输入一行，回车保存。"
      : "这个项目还没有任务。在下面输入一行，回车保存。";

  // 「待办」没有模块，也就没有分组行：表格直接平铺。
  if (state.container.kind === "todo") {
    const parents = state.tasks.filter((task) => task.parentTaskId === null);
    for (const task of sortTasks(parents)) dom.body.append(renderTaskRow(task));
    return;
  }

  for (const group of buildGroups(state.tasks, state.modules)) {
    renderGroup(group, dom.body);
  }
}

/** 深度优先：分组行 → 组内任务 → 子分组（缩进一级）。 */
function renderGroup(group: Group, host: HTMLElement, collapsed = false): void {
  const { done, total } = countGroup(group);
  const row = el("tr", group.depth === 1 ? "row-subgroup" : "row-group");
  const cell = el("td");
  cell.colSpan = 5;

  const label = el("span", "group-label");
  label.style.paddingLeft = `${group.depth * 20}px`;
  label.append(el("span", "group-caret", collapsed ? "▸" : "▾"), el("span", undefined, group.label));

  if (group.children.length > 0) {
    label.append(el("span", "group-meta", `${group.children.length} 个子模块`));
  }
  label.append(el("span", "group-count", `${done}/${total}`));

  cell.append(label);
  row.append(cell);
  host.append(row);

  if (collapsed) return;

  for (const task of group.tasks) host.append(renderTaskRow(task));
  for (const child of group.children) renderGroup(child, host);
}

function renderTaskRow(task: Task): HTMLTableRowElement {
  const row = el("tr", "row-task");
  row.dataset.taskId = String(task.id);
  if (task.status === "done") row.classList.add("is-done");

  row.append(
    renderStatusCell(task),
    renderPriorityCell(task),
    renderTitleCell(task),
    renderNoteCell(task),
    el("td"), // 「…」菜单列留给归类/删除命令，本切片还没有对应命令，先不放假按钮
  );
  return row;
}

function renderStatusCell(task: Task): HTMLTableCellElement {
  const cell = el("td");
  const labels: Record<Task["status"], string> = {
    todo: "未完成",
    doing: "进行中",
    done: "已完成",
  };
  const icon = el("span", `status status-${task.status}`);
  // 状态格点击循环三态要等 `plan_set_status` / `apply_set_status` 落地，
  // 这里只如实显示状态，不放一个点了没反应的控件。
  icon.title = labels[task.status];
  cell.append(icon);
  return cell;
}

function renderPriorityCell(task: Task): HTMLTableCellElement {
  const cell = el("td");
  const labels: Record<Task["priority"], string> = { high: "高", mid: "中", low: "低" };
  const chip = el("span", `priority priority-${task.priority}`, labels[task.priority]);
  cell.append(chip);
  return cell;
}

function renderTitleCell(task: Task): HTMLTableCellElement {
  const cell = el("td", "cell-title");
  cell.append(el("span", "task-title", task.title));
  return cell;
}

function renderNoteCell(task: Task): HTMLTableCellElement {
  const cell = el("td", "cell-note");
  const note = task.note.trim();
  cell.append(el("span", note ? "note" : "note is-empty", note || "—"));
  return cell;
}

// ---------------------------------------------------------------------------
// 事件
// ---------------------------------------------------------------------------

dom.projectAdd.addEventListener("click", () => {
  dom.projectForm.hidden = false;
  dom.projectInput.focus();
});

dom.projectForm.addEventListener("submit", (event) => {
  event.preventDefault();
  void createProject();
});

dom.moduleAdd.addEventListener("click", () => {
  dom.moduleAdd.hidden = true;
  dom.moduleForm.hidden = false;
  dom.moduleInput.focus();
});

dom.moduleForm.addEventListener("submit", (event) => {
  event.preventDefault();
  void createModule();
});

dom.taskForm.addEventListener("submit", (event) => {
  event.preventDefault();
  void createTask();
});

async function createProject(): Promise<void> {
  const name = dom.projectInput.value.trim();
  if (!name) return;
  try {
    const created = await api.createProject(name);
    dom.projectInput.value = "";
    dom.projectForm.hidden = true;
    if (created.id !== null) state.container = { kind: "project", id: created.id };
    await reload();
  } catch (error) {
    showNotice(describe(error), "error");
  }
}

async function createModule(): Promise<void> {
  if (state.container.kind !== "project") return;
  const container = state.container;
  const name = dom.moduleInput.value.trim();
  if (!name) return;
  try {
    await api.createModule(container.id, name);
    closeModuleForm();
    await reload();
  } catch (error) {
    showNotice(describe(error), "error");
  }
}

function closeModuleForm(): void {
  dom.moduleInput.value = "";
  dom.moduleForm.hidden = true;
  dom.moduleAdd.hidden = state.container.kind !== "project";
}

async function createTask(): Promise<void> {
  const title = dom.taskInput.value.trim();
  if (!title) return;
  try {
    await api.createTask(state.container, title);
    dom.taskInput.value = "";
    await reload();
  } catch (error) {
    showNotice(describe(error), "error");
  }
}

// ---------------------------------------------------------------------------
// 轻提示
// ---------------------------------------------------------------------------

let noticeTimer: number | undefined;

/** 底部一条会自己消失的提示，不阻塞任何操作。 */
function showNotice(message: string, kind: "info" | "error" = "info"): void {
  dom.notice.textContent = message;
  dom.notice.dataset.kind = kind;
  dom.notice.hidden = false;

  window.clearTimeout(noticeTimer);
  noticeTimer = window.setTimeout(() => {
    dom.notice.hidden = true;
  }, kind === "error" ? 8000 : 3000);
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

window.addEventListener("DOMContentLoaded", () => {
  render();
  void reload();
});

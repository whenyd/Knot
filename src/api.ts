/**
 * 唯一的前端 ↔ 后端接口层。
 *
 * 约定：`invoke` 只出现在这个文件里。别处的代码只认这里的类型和函数，
 * 命令改名时只有一处要改。字段名是 camelCase——Rust 侧用
 * `#[serde(rename_all = "camelCase")]` 对齐。
 */
import { invoke } from "@tauri-apps/api/core";

/** 任务所在的容器：虚拟的「我的待办」，或某个真实项目。 */
export type Container = { kind: "todo" } | { kind: "project"; id: number };

export type Status = "todo" | "doing" | "done";
export type Priority = "high" | "mid" | "low";

/** 可整组删除的分类节点。模块和子模块的 id 各自独立，必须带 kind。 */
export type NodeKind = "module" | "submodule";

/** 新任务的归属。子模块必项连同它的模块一起给，后端会校验这层父子关系。 */
export interface ScopeTarget {
  moduleId: number | null;
  submoduleId: number | null;
}

/** 移动目标：容器 + 两级分类。字段名与 Rust 的 `MoveTarget` 逐字对应。 */
export interface MoveTarget extends ScopeTarget {
  container: Container;
}

export interface Project {
  /** 「我的待办」没有 id —— 它不是数据库里的一行。 */
  id: number | null;
  name: string;
  isTodo: boolean;
  taskCount: number;
}

export interface Submodule {
  id: number;
  name: string;
}

export interface Module {
  id: number;
  name: string;
  submodules: Submodule[];
}

export interface Task {
  id: number;
  projectId: number | null;
  moduleId: number | null;
  submoduleId: number | null;
  parentTaskId: number | null;
  title: string;
  status: Status;
  priority: Priority;
  note: string;
  sortOrder: number;
}

/** `plan_set_status` 的返回：改之前要告诉界面什么。 */
export interface StatusPlan {
  /** 会受影响的任务（目标任务 + 需要一并改的子任务）。 */
  affects: Task[];
  needsConfirm: boolean;
  /** 需要确认时给用户看的一句话；否则为空串。 */
  reason: string;
}

/** `apply_set_status` 的返回。 */
export interface StatusOutcome {
  changed: Task[];
  /** 父任务被自动改动时的轻提示。 */
  notices: string[];
}

/** 删除前的告知：这次会删掉多少个任务（含子任务）。 */
export interface DeletePlan {
  taskCount: number;
}

/** `create_task` 的返回：任务本体 + 父任务被聚合自动改动时的轻提示。 */
export interface TaskCreated {
  task: Task;
  notices: string[];
}

/** `delete_task` 的返回：删掉子任务后父任务被自动改动时的轻提示。 */
export interface DeleteOutcome {
  notices: string[];
}

export const api = {
  listProjects: () => invoke<Project[]>("list_projects"),
  createProject: (name: string) => invoke<Project>("create_project", { name }),
  listModules: (projectId: number) => invoke<Module[]>("list_modules", { projectId }),
  createModule: (projectId: number, name: string) =>
    invoke<Module>("create_module", { projectId, name }),
  createSubmodule: (moduleId: number, name: string) =>
    invoke<Submodule>("create_submodule", { moduleId, name }),
  renameProject: (id: number, name: string) => invoke<Project>("rename_project", { id, name }),
  renameModule: (id: number, name: string) => invoke<Module>("rename_module", { id, name }),
  renameSubmodule: (id: number, name: string) =>
    invoke<Submodule>("rename_submodule", { id, name }),
  listTasks: (container: Container) => invoke<Task[]>("list_tasks", { container }),
  createTask: (
    container: Container,
    target: ScopeTarget,
    title: string,
    parentTaskId: number | null = null,
  ) => invoke<TaskCreated>("create_task", { container, ...target, parentTaskId, title }),
  renameTask: (id: number, title: string) => invoke<Task>("rename_task", { id, title }),
  setNote: (id: number, note: string) => invoke<Task>("set_note", { id, note }),
  moveTask: (id: number, target: MoveTarget) => invoke<Task>("move_task", { id, target }),
  setPriority: (id: number, priority: Priority) =>
    invoke<Task>("set_priority", { id, priority }),
  /** 第一步：只算要不要确认，不写库。 */
  planSetStatus: (id: number, status: Status) =>
    invoke<StatusPlan>("plan_set_status", { id, status }),
  /** 第二步：写库。`cascade` 为真时连同未完成子任务一起标为已完成。 */
  applySetStatus: (id: number, status: Status, cascade: boolean) =>
    invoke<StatusOutcome>("apply_set_status", { id, status, cascade }),
  /** 删除任务的第一步：只数会删掉几条（含子任务），不写库。 */
  planDeleteTask: (id: number) => invoke<DeletePlan>("plan_delete_task", { id }),
  deleteTask: (id: number) => invoke<DeleteOutcome>("delete_task", { id }),
  /** 删除模块 / 子模块的第一步：只数会删掉几条（含子模块里的任务），不写库。 */
  planDeleteNode: (kind: NodeKind, id: number) =>
    invoke<DeletePlan>("plan_delete_node", { kind, id }),
  deleteNode: (kind: NodeKind, id: number) => invoke<void>("delete_node", { kind, id }),
  /** 删除项目的第一步：项目里的全部任务都会一起删除。 */
  planDeleteProject: (id: number) => invoke<DeletePlan>("plan_delete_project", { id }),
  deleteProject: (id: number) => invoke<void>("delete_project", { id }),
  /** 仅开发版注册：删除活库并重建一个空库。正式版调用会失败。 */
  resetDatabase: () => invoke<void>("reset_database"),
};

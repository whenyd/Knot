/**
 * The single frontend ↔ backend interface layer.
 *
 * Rule: `invoke` appears only in this file. Other code knows only the types and
 * functions here, so renaming a command means editing one place. Field names are
 * camelCase — the Rust side matches them with
 * `#[serde(rename_all = "camelCase")]`.
 */
import { invoke } from "@tauri-apps/api/core";

/** The container a task lives in: the virtual "My tasks", or a real project. */
export type Container = { kind: "todo" } | { kind: "project"; id: number };

export type Status = "todo" | "doing" | "done";
export type Priority = "high" | "mid" | "low";

/** A backend message: a stable key plus typed parameters, translated by the UI. */
export interface BackendMessage {
  key: string;
  params?: Record<string, MessageParam>;
}

export type MessageParam = string | number | BackendMessage;

/** A classification node deletable as a whole. Module and submodule ids are separate, so kind is required. */
export type NodeKind = "module" | "submodule";

/** Ownership of a new task. A submodule must come with its module; the backend validates that parent-child relationship. */
export interface ScopeTarget {
  moduleId: number | null;
  submoduleId: number | null;
}

/** Move target: container + two classification levels. Field names match Rust's `MoveTarget` exactly. */
export interface MoveTarget extends ScopeTarget {
  container: Container;
}

export interface Project {
  /** "My tasks" has no id — it is not a row in the database. */
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
  /** Local time `YYYY-MM-DDTHH:MM`; written on creation, null for tasks from before the upgrade. */
  createdAt: string | null;
  /** Local time `YYYY-MM-DDTHH:MM`, null = not recorded; maintained automatically by status changes. */
  startAt: string | null;
  endAt: string | null;
  sortOrder: number;
}

/** Return of `plan_set_status`: what the UI must be told before the change. */
export interface StatusPlan {
  /** Tasks affected (the target task plus subtasks that must change with it). */
  affects: Task[];
  needsConfirm: boolean;
  /** Structured reason shown only when `needsConfirm` is true. */
  reason: BackendMessage | null;
}

/** Return of `apply_set_status`. */
export interface StatusOutcome {
  changed: Task[];
  /** Toast when a parent task was changed automatically. */
  notices: BackendMessage[];
}

/** Pre-delete notice: how many tasks this will delete (including subtasks). */
export interface DeletePlan {
  taskCount: number;
}

/** Return of `create_task`: the task itself plus toasts when a parent was changed by aggregation. */
export interface TaskCreated {
  task: Task;
  notices: BackendMessage[];
}

/** Return of `delete_task`: toasts when a parent was changed automatically after deleting a subtask. */
export interface DeleteOutcome {
  notices: BackendMessage[];
}

/** Return of `reparent_task`: the task after re-parenting plus toasts when a parent was changed automatically. */
export interface ReparentOutcome {
  task: Task;
  notices: BackendMessage[];
}

export const api = {
  listProjects: () => invoke<Project[]>("list_projects"),
  createProject: (name: string) => invoke<Project>("create_project", { name }),
  listModules: (projectId: number) => invoke<Module[]>("list_modules", { projectId }),
  createModule: (projectId: number, name: string) =>
    invoke<Module>("create_module", { projectId, name }),
  createSubmodule: (moduleId: number, name: string) =>
    invoke<Submodule>("create_submodule", { moduleId, name }),
  /** Reorder modules: insert before `beforeId`; `null` = move to the end. Returns the new full module tree. */
  moveModule: (id: number, beforeId: number | null) =>
    invoke<Module[]>("move_module", { id, beforeId }),
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
  /** `parentTaskId` null promotes the task to top level; otherwise it becomes a subtask of that parent. */
  reparentTask: (id: number, parentTaskId: number | null) =>
    invoke<ReparentOutcome>("reparent_task", { id, parentTaskId }),
  setPriority: (id: number, priority: Priority) =>
    invoke<Task>("set_priority", { id, priority }),
  /** Step one: only compute whether confirmation is needed, no DB write. */
  planSetStatus: (id: number, status: Status) =>
    invoke<StatusPlan>("plan_set_status", { id, status }),
  /** Step two: write to the DB. When `cascade` is true, unfinished subtasks are marked done too. */
  applySetStatus: (id: number, status: Status, cascade: boolean) =>
    invoke<StatusOutcome>("apply_set_status", { id, status, cascade }),
  /** First delete step: only count how many rows would go (including subtasks), no DB write. */
  planDeleteTask: (id: number) => invoke<DeletePlan>("plan_delete_task", { id }),
  deleteTask: (id: number) => invoke<DeleteOutcome>("delete_task", { id }),
  /** First module / submodule delete step: only count how many rows would go (including tasks inside), no DB write. */
  planDeleteNode: (kind: NodeKind, id: number) =>
    invoke<DeletePlan>("plan_delete_node", { kind, id }),
  deleteNode: (kind: NodeKind, id: number) => invoke<void>("delete_node", { kind, id }),
  /** First project delete step: all tasks in the project are deleted too. */
  planDeleteProject: (id: number) => invoke<DeletePlan>("plan_delete_project", { id }),
  deleteProject: (id: number) => invoke<void>("delete_project", { id }),
  /** Registered in dev builds only: delete the live DB and recreate an empty one. Calls fail in release builds. */
  resetDatabase: () => invoke<void>("reset_database"),
};

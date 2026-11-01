/**
 * 唯一的前端 ↔ 后端接口层。
 *
 * 约定：`invoke` 只出现在这个文件里。别处的代码只认这里的类型和函数，
 * 命令改名时只有一处要改。字段名是 camelCase——Rust 侧用
 * `#[serde(rename_all = "camelCase")]` 对齐。
 */
import { invoke } from "@tauri-apps/api/core";

/** 任务所在的容器：虚拟的「待办」，或某个真实项目。 */
export type Container = { kind: "todo" } | { kind: "project"; id: number };

export type Status = "todo" | "doing" | "done";
export type Priority = "high" | "mid" | "low";

export interface Project {
  /** 「待办」没有 id —— 它不是数据库里的一行。 */
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

export const api = {
  listProjects: () => invoke<Project[]>("list_projects"),
  createProject: (name: string) => invoke<Project>("create_project", { name }),
  listModules: (projectId: number) => invoke<Module[]>("list_modules", { projectId }),
  createModule: (projectId: number, name: string) =>
    invoke<Module>("create_module", { projectId, name }),
  listTasks: (container: Container) => invoke<Task[]>("list_tasks", { container }),
  createTask: (container: Container, title: string) =>
    invoke<Task>("create_task", { container, title }),
};

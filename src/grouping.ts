/**
 * 数据管线里"算"的那一段：排序、分组。纯函数，不碰 DOM、不发命令。
 *
 * 之所以放在前端而不是 SQL：任务量在本机是小规模，前端算足够快，
 * 也免得为"分组 × 筛选 × 排序"的每种组合写一条查询。
 */
import type { Module, Priority, Task } from "./api";

/** 组内排序：优先级 高 → 中 → 低，同优先级按添加顺序（新的在下）。 */
const PRIORITY_RANK: Record<Priority, number> = { high: 0, mid: 1, low: 2 };

export function sortTasks(tasks: readonly Task[]): Task[] {
  return [...tasks].sort(
    (a, b) => PRIORITY_RANK[a.priority] - PRIORITY_RANK[b.priority] || a.sortOrder - b.sortOrder,
  );
}

/**
 * 一个分组（模块、子模块，或「未分类」）。
 *
 * 分组行与任务行是同一种"行"，区别只在 `depth` 和有没有 `tasks`——
 * 这样渲染器只需要走一遍树的深度优先。
 */
export interface Group {
  key: string;
  label: string;
  /** 0 = 模块；1 = 子模块（缩进一级）。 */
  depth: 0 | 1;
  tasks: Task[];
  children: Group[];
}

/**
 * 按模块建树分组。位置即分类：模块/子模块不占列，只当分组行。
 *
 * 任务归属优先级：`submoduleId` > `moduleId` > 未分类。子任务不单独成组，
 * 由渲染器跟在父任务下面（本切片还不建子任务）。
 */
export function buildGroups(tasks: readonly Task[], modules: readonly Module[]): Group[] {
  const store = new Map<string, Task[]>();
  const push = (key: string, task: Task) => {
    const bucket = store.get(key);
    if (bucket) bucket.push(task);
    else store.set(key, [task]);
  };

  for (const task of tasks) {
    if (task.parentTaskId !== null) continue; // 子任务跟随父任务，不占分组
    if (task.submoduleId !== null) push(`submodule-${task.submoduleId}`, task);
    else if (task.moduleId !== null) push(`module-${task.moduleId}`, task);
    else push("unclassified", task);
  }

  const take = (key: string) => sortTasks(store.get(key) ?? []);

  // 模块分组一律保留，即使暂时是 0/0：本切片还没有"移动到模块"的命令，
  // 若把空模块藏起来，新建模块就会看起来毫无反应。
  const groups: Group[] = modules.map((module) => ({
    key: `module-${module.id}`,
    label: module.name,
    depth: 0,
    tasks: take(`module-${module.id}`),
    children: module.submodules.map((submodule) => ({
      key: `submodule-${submodule.id}`,
      label: submodule.name,
      depth: 1,
      tasks: take(`submodule-${submodule.id}`),
      children: [],
    })),
  }));

  // 「未分类」是固定分组，排最后；没有任务就不占一行。
  const unclassified = take("unclassified");
  if (unclassified.length > 0) {
    groups.push({
      key: "unclassified",
      label: "未分类",
      depth: 0,
      tasks: unclassified,
      children: [],
    });
  }

  return groups;
}

/** 分组行计数：只算父任务。`done` / `total`。 */
export function countGroup(group: Group): { done: number; total: number } {
  let done = 0;
  let total = 0;
  const walk = (node: Group) => {
    for (const task of node.tasks) {
      total += 1;
      if (task.status === "done") done += 1;
    }
    node.children.forEach(walk);
  };
  walk(group);
  return { done, total };
}

/**
 * 数据管线里"算"的那一段：排序、分组、范围筛选。纯函数，不碰 DOM、不发命令。
 *
 * 之所以放在前端而不是 SQL：任务量在本机是小规模，前端算足够快，
 * 也免得为"分组 × 筛选 × 排序"的每种组合写一条查询。
 */
import type { Module, Priority, ScopeTarget, Status, Task } from "./api";

/** 状态与优先级是“且”关系；集合内是“或”关系。 */
export interface TaskFilters {
  statuses: ReadonlySet<Status>;
  priorities: ReadonlySet<Priority>;
}

export function taskMatchesFilters(task: Task, filters: TaskFilters): boolean {
  return filters.statuses.has(task.status) && filters.priorities.has(task.priority);
}

/** 筛选只决定渲染哪些任务，不改动分组树和计数所用的原始任务。 */
export function filterTasks(tasks: readonly Task[], filters: TaskFilters): Task[] {
  return tasks.filter((task) => taskMatchesFilters(task, filters));
}

/** 组内排序：优先级 高 → 中 → 低，同优先级按添加顺序（新的在下）。 */
const PRIORITY_RANK: Record<Priority, number> = { high: 0, mid: 1, low: 2 };

export function sortTasks(tasks: readonly Task[]): Task[] {
  return [...tasks].sort(
    (a, b) => PRIORITY_RANK[a.priority] - PRIORITY_RANK[b.priority] || a.sortOrder - b.sortOrder,
  );
}

/**
 * 主视图的范围：整个项目 / 某个模块 / 某个子模块 / 默认。
 *
 * 「我的待办」没有这层树，所以它不出现在这里——我的待办视图恒为整个容器。
 */
export type Scope =
  | { kind: "project" }
  | { kind: "module"; id: number }
  | { kind: "submodule"; id: number }
  | { kind: "unclassified" };

/** 一个分组（模块、子模块，或「默认」）。 */
export interface Group {
  kind: "module" | "submodule" | "unclassified";
  /** 「默认」是派生分组，没有 id。 */
  id: number | null;
  label: string;
  /** 0 = 模块（或提升为范围的子模块）；1 = 缩进一级的子模块。 */
  depth: 0 | 1;
  tasks: Task[];
  children: Group[];
}

/**
 * 按模块建树分组。位置即分类：模块/子模块不占列，只当分组行。
 *
 * 任务归属优先级：`submoduleId` > `moduleId` > 默认。子任务不单独成组，
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

  // 「默认」是派生分组，空时不占一行；未归类任务最需要先看到，所以排在模块之前。
  const groups: Group[] = [];
  const unclassified = take("unclassified");
  if (unclassified.length > 0) {
    groups.push({
      kind: "unclassified",
      id: null,
      label: "默认",
      depth: 0,
      tasks: unclassified,
      children: [],
    });
  }

  // 用户建的模块与子模块一律保留，即使暂时是 0/0：它们是结构，
  // 藏起来会让人以为没建成。
  const moduleGroups: Group[] = modules.map((module) => ({
    kind: "module",
    id: module.id,
    label: module.name,
    depth: 0,
    tasks: take(`module-${module.id}`),
    children: module.submodules.map((submodule) => ({
      kind: "submodule",
      id: submodule.id,
      label: submodule.name,
      depth: 1,
      tasks: take(`submodule-${submodule.id}`),
      children: [],
    })),
  }));
  groups.push(...moduleGroups);

  return groups;
}

/** 分组行计数：只算父任务。`done` / `total`，含子分组。 */
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

/** 一组分组的合计计数。 */
export function countGroups(groups: readonly Group[]): { done: number; total: number } {
  return groups.reduce(
    (sum, group) => {
      const { done, total } = countGroup(group);
      return { done: sum.done + done, total: sum.total + total };
    },
    { done: 0, total: 0 },
  );
}

/**
 * 把范围套到分组树上。
 *
 * 范围的根就是视图的根：选中某个子模块时它自己成了最外层，不再缩进；
 * 选中模块时保留它的子模块（子树的形状不变）。
 */
export function selectScope(groups: readonly Group[], scope: Scope): Group[] {
  switch (scope.kind) {
    case "project":
      return [...groups];
    case "module":
      return groups.filter((group) => group.kind === "module" && group.id === scope.id);
    case "unclassified":
      return groups.filter((group) => group.kind === "unclassified");
    case "submodule": {
      for (const group of groups) {
        const child = group.children.find(
          (node) => node.kind === "submodule" && node.id === scope.id,
        );
        if (child) return [{ ...child, depth: 0 }];
      }
      return [];
    }
  }
}

/** 范围在左栏的显示名。 */
export function scopeLabel(groups: readonly Group[], scope: Scope): string {
  switch (scope.kind) {
    case "project":
      return "全部";
    case "unclassified":
      return "默认";
    case "module":
      return groups.find((g) => g.kind === "module" && g.id === scope.id)?.label ?? "模块";
    case "submodule":
      for (const group of groups) {
        const child = group.children.find((n) => n.kind === "submodule" && n.id === scope.id);
        if (child) return child.label;
      }
      return "子模块";
  }
}

/**
 * 当前范围对应的任务归属：新建任务就落在这里。
 *
 * “全部”与“默认”都是没有分类的位置，新任务自然进默认。
 */
export function resolveTarget(groups: readonly Group[], scope: Scope): ScopeTarget {
  switch (scope.kind) {
    case "project":
    case "unclassified":
      return { moduleId: null, submoduleId: null };
    case "module":
      return { moduleId: scope.id, submoduleId: null };
    case "submodule": {
      // 子模块必须连同它的模块一起报上去
      for (const group of groups) {
        if (group.kind !== "module" || group.id === null) continue;
        const owns = group.children.some(
          (child) => child.kind === "submodule" && child.id === scope.id,
        );
        if (owns) return { moduleId: group.id, submoduleId: scope.id };
      }
      return { moduleId: null, submoduleId: null };
    }
  }
}

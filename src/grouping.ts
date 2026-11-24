/**
 * The "compute" half of the data pipeline: sorting, grouping, scope filtering.
 * Pure functions: no DOM, no commands.
 *
 * It lives in the frontend rather than SQL because the task count is small on
 * one machine and the frontend is fast enough, and it avoids writing a query
 * for every combination of grouping × filtering × sorting.
 */
import type { Module, Priority, ScopeTarget, Status, Task } from "./api";

/** Status and priority combine with AND; values within a set combine with OR. */
export interface TaskFilters {
  statuses: ReadonlySet<Status>;
  priorities: ReadonlySet<Priority>;
}

export function taskMatchesFilters(task: Task, filters: TaskFilters): boolean {
  return filters.statuses.has(task.status) && filters.priorities.has(task.priority);
}

/** Filtering only decides which tasks render; the grouping tree and counts keep the original tasks. */
export function filterTasks(tasks: readonly Task[], filters: TaskFilters): Task[] {
  return tasks.filter((task) => taskMatchesFilters(task, filters));
}

/** Priority sort value: high → mid → low. */
const PRIORITY_RANK: Record<Priority, number> = { high: 0, mid: 1, low: 2 };

/** Legacy tasks without a creation time sort as oldest; the string is `YYYY-MM-DDTHH:MM`, so compare directly. */
function compareCreatedAt(a: Task, b: Task): number {
  const aTime = a.createdAt ?? "";
  const bTime = b.createdAt ?? "";
  if (aTime === bTime) return 0;
  return aTime < bTime ? -1 : 1;
}

function doneRank(task: Task): number {
  return task.status === "done" ? 1 : 0;
}

/** Columns whose header can be clicked to sort. */
export type SortField = "priority" | "created" | "start" | "end";
export type SortDirection = "asc" | "desc";

/** `null` means the default order: status → priority → creation time. */
export interface TaskSort {
  field: SortField;
  direction: SortDirection;
}

/** Unrecorded times sort after recorded ones in both directions. */
function compareTime(a: string | null, b: string | null, direction: number): number {
  if (a === null || b === null) {
    if (a === b) return 0;
    return a === null ? 1 : -1;
  }
  const cmp = a < b ? -1 : a > b ? 1 : 0;
  return cmp * direction;
}

function compareField(a: Task, b: Task, field: SortField, direction: number): number {
  switch (field) {
    case "priority":
      return (PRIORITY_RANK[a.priority] - PRIORITY_RANK[b.priority]) * direction;
    case "created":
      return compareTime(a.createdAt, b.createdAt, direction);
    case "start":
      return compareTime(a.startAt, b.startAt, direction);
    case "end":
      return compareTime(a.endAt, b.endAt, direction);
  }
}

/**
 * Sort within a group.
 *
 * Without a column sort: unfinished / in-progress first, done last; inside each
 * part, by priority → creation time.
 * With a column selected: the whole group sorts by that column (done tasks are
 * no longer separated), with ties broken by creation time / insertion order.
 */
export function sortTasks(tasks: readonly Task[], sort: TaskSort | null = null): Task[] {
  const direction = sort?.direction === "desc" ? -1 : 1;
  return [...tasks].sort((a, b) => {
    if (sort !== null) {
      const primary = compareField(a, b, sort.field, direction);
      if (primary !== 0) return primary;
      return compareCreatedAt(a, b) || a.sortOrder - b.sortOrder || a.id - b.id;
    }
    return (
      doneRank(a) - doneRank(b) ||
      PRIORITY_RANK[a.priority] - PRIORITY_RANK[b.priority] ||
      compareCreatedAt(a, b) ||
      a.sortOrder - b.sortOrder ||
      a.id - b.id
    );
  });
}

/**
 * Order of candidate parents for "demote to subtask": unfinished / in-progress
 * first, done last; within a status, by creation time ascending (legacy tasks
 * without a creation time count as the oldest).
 */
export function sortParentCandidates(tasks: readonly Task[]): Task[] {
  return [...tasks].sort(
    (a, b) =>
      doneRank(a) - doneRank(b) ||
      compareCreatedAt(a, b) ||
      a.sortOrder - b.sortOrder ||
      a.id - b.id,
  );
}

/**
 * The main view's scope: whole project / one module / one submodule / default.
 *
 * "My tasks" has no tree, so it never appears here — its view is always the
 * whole container.
 */
export type Scope =
  | { kind: "project" }
  | { kind: "module"; id: number }
  | { kind: "submodule"; id: number }
  | { kind: "unclassified" };

/** A group (module, submodule, or "unclassified"). */
export interface Group {
  kind: "module" | "submodule" | "unclassified";
  /** The default group is derived and has no id. */
  id: number | null;
  /** User-defined name; the derived default group has none. */
  label: string | null;
  /** 0 = module (or a submodule promoted to scope); 1 = submodule indented one level. */
  depth: 0 | 1;
  tasks: Task[];
  children: Group[];
}

/**
 * Build the grouping tree by module. Location is the classification:
 * modules/submodules take no column, only group rows.
 *
 * Ownership priority: `submoduleId` > `moduleId` > default. Subtasks never form
 * their own group; the renderer shows them under their parent (this slice does
 * not create subtasks yet).
 */
export function buildGroups(
  tasks: readonly Task[],
  modules: readonly Module[],
  sort: TaskSort | null = null,
): Group[] {
  const store = new Map<string, Task[]>();
  const push = (key: string, task: Task) => {
    const bucket = store.get(key);
    if (bucket) bucket.push(task);
    else store.set(key, [task]);
  };

  for (const task of tasks) {
    if (task.parentTaskId !== null) continue; // subtasks follow their parent, no group of their own
    if (task.submoduleId !== null) push(`submodule-${task.submoduleId}`, task);
    else if (task.moduleId !== null) push(`module-${task.moduleId}`, task);
    else push("unclassified", task);
  }

  const take = (key: string) => sortTasks(store.get(key) ?? [], sort);

  // The default group is derived and takes no row when empty; unclassified tasks
  // need to be seen first, so it goes before the modules.
  const groups: Group[] = [];
  const unclassified = take("unclassified");
  if (unclassified.length > 0) {
    groups.push({
      kind: "unclassified",
      id: null,
      label: null,
      depth: 0,
      tasks: unclassified,
      children: [],
    });
  }

  // User-created modules and submodules are always kept, even at 0/0 for now:
  // they are structure, and hiding them would look like they were never created.
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

/** Group row count: parent tasks only. `done` / `total`, including child groups. */
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

/** Combined count across a list of groups. */
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
 * Apply a scope to the grouping tree.
 *
 * The scope's root is the view's root: a selected submodule becomes the
 * outermost level and is no longer indented; a selected module keeps its
 * submodules (the subtree keeps its shape).
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

/**
 * Task ownership for the current scope: a new task lands here.
 *
 * "All" and "default" are both positions without a classification, so new tasks
 * naturally fall into the default group.
 */
export function resolveTarget(groups: readonly Group[], scope: Scope): ScopeTarget {
  switch (scope.kind) {
    case "project":
    case "unclassified":
      return { moduleId: null, submoduleId: null };
    case "module":
      return { moduleId: scope.id, submoduleId: null };
    case "submodule": {
      // A submodule must be reported together with its module
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

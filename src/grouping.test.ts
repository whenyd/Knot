import { describe, expect, it } from "vitest";

import type { Module, Priority, Status, Task } from "./api";
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
} from "./grouping";

function makeTask({ id, ...overrides }: Partial<Task> & Pick<Task, "id">): Task {
  return {
    id,
    projectId: 1,
    moduleId: null,
    submoduleId: null,
    parentTaskId: null,
    title: `任务 ${id}`,
    status: "todo",
    priority: "mid",
    note: "",
    createdAt: null,
    startAt: null,
    endAt: null,
    sortOrder: 0,
    ...overrides,
  };
}

function makeModule(
  id: number,
  name: string,
  submodules: Module["submodules"] = [],
): Module {
  return { id, name, submodules };
}

describe("sortTasks", () => {
  it("sorts by priority, then creation time ascending, without mutating the input array", () => {
    const low = makeTask({ id: 1, priority: "low", createdAt: "2026-01-01T08:00", sortOrder: 1 });
    const highLater = makeTask({
      id: 2,
      priority: "high",
      createdAt: "2026-01-01T08:00",
      sortOrder: 9,
    });
    const mid = makeTask({ id: 3, priority: "mid", createdAt: "2026-01-02T08:00", sortOrder: 2 });
    const highEarlier = makeTask({
      id: 4,
      priority: "high",
      createdAt: "2025-12-01T08:00",
      sortOrder: 3,
    });
    const tasks = [low, highLater, mid, highEarlier];
    const before = [...tasks];

    expect(sortTasks(tasks).map((task) => task.id)).toEqual([4, 2, 3, 1]);
    expect(tasks).toEqual(before);
  });

  it("puts the whole done group last, still sorted by priority → creation time inside", () => {
    const doneHigh = makeTask({
      id: 1,
      status: "done",
      priority: "high",
      createdAt: "2025-01-01T08:00",
    });
    const todoLow = makeTask({
      id: 2,
      status: "todo",
      priority: "low",
      createdAt: "2026-01-01T08:00",
    });
    const doingMid = makeTask({
      id: 3,
      status: "doing",
      priority: "mid",
      createdAt: "2026-02-01T08:00",
    });
    const doneLow = makeTask({
      id: 4,
      status: "done",
      priority: "low",
      createdAt: "2025-06-01T08:00",
    });

    expect(sortTasks([doneHigh, todoLow, doingMid, doneLow]).map((task) => task.id)).toEqual([
      3, 2, 1, 4,
    ]);
  });

  it("treats a missing creation time as oldest; on equal times falls back to insertion order and id", () => {
    const legacy = makeTask({ id: 5, priority: "high", createdAt: null, sortOrder: 9 });
    const laterAdd = makeTask({
      id: 6,
      priority: "high",
      createdAt: "2026-01-01T08:00",
      sortOrder: 4,
    });
    const earlierAdd = makeTask({
      id: 7,
      priority: "high",
      createdAt: "2026-01-01T08:00",
      sortOrder: 2,
    });

    expect(sortTasks([laterAdd, legacy, earlierAdd]).map((task) => task.id)).toEqual([5, 7, 6]);
  });
});

describe("sortTasks (column sorting)", () => {
  it("sorts by priority ascending / descending and no longer moves done tasks last", () => {
    const doneHigh = makeTask({
      id: 1,
      status: "done",
      priority: "high",
      createdAt: "2025-01-01T08:00",
    });
    const todoLow = makeTask({ id: 2, priority: "low", createdAt: "2026-01-01T08:00" });
    const doingMid = makeTask({
      id: 3,
      status: "doing",
      priority: "mid",
      createdAt: "2026-02-01T08:00",
    });
    const tasks = [doneHigh, todoLow, doingMid];

    expect(
      sortTasks(tasks, { field: "priority", direction: "asc" }).map((task) => task.id),
    ).toEqual([1, 3, 2]);
    expect(
      sortTasks(tasks, { field: "priority", direction: "desc" }).map((task) => task.id),
    ).toEqual([2, 3, 1]);
  });

  it("sorts by start time both ways; tasks without a time always go last", () => {
    const early = makeTask({ id: 1, startAt: "2026-01-01T08:00" });
    const late = makeTask({ id: 2, startAt: "2026-03-01T08:00" });
    const none = makeTask({ id: 3, startAt: null });

    expect(
      sortTasks([late, none, early], { field: "start", direction: "asc" }).map((task) => task.id),
    ).toEqual([1, 2, 3]);
    expect(
      sortTasks([early, none, late], { field: "start", direction: "desc" }).map((task) => task.id),
    ).toEqual([2, 1, 3]);
  });

  it("falls back to creation time and insertion order on equal sort values", () => {
    const later = makeTask({ id: 1, createdAt: "2026-02-01T08:00", sortOrder: 5 });
    const earlier = makeTask({ id: 2, createdAt: "2026-01-01T08:00", sortOrder: 9 });
    const sameMinute = makeTask({ id: 3, createdAt: "2026-02-01T08:00", sortOrder: 2 });

    expect(
      sortTasks([later, earlier, sameMinute], { field: "created", direction: "desc" }).map(
        (task) => task.id,
      ),
    ).toEqual([3, 1, 2]);
  });
});

describe("sortParentCandidates", () => {
  it("orders unfinished / in-progress before done; within a status, by creation time ascending", () => {
    const doneEarly = makeTask({ id: 1, status: "done", createdAt: "2026-01-01T08:00", sortOrder: 1 });
    const todoLate = makeTask({ id: 2, status: "todo", createdAt: "2026-03-01T08:00", sortOrder: 2 });
    const doingMid = makeTask({ id: 3, status: "doing", createdAt: "2026-02-01T08:00", sortOrder: 3 });
    const doneLate = makeTask({ id: 4, status: "done", createdAt: "2026-04-01T08:00", sortOrder: 4 });
    const tasks = [doneEarly, todoLate, doingMid, doneLate];
    const before = [...tasks];

    expect(sortParentCandidates(tasks).map((task) => task.id)).toEqual([3, 2, 1, 4]);
    expect(tasks).toEqual(before);
  });

  it("falls back to insertion order and then id on equal (or missing) creation times", () => {
    const later = makeTask({ id: 5, createdAt: "2026-02-01T08:00", sortOrder: 9 });
    const earlier = makeTask({ id: 6, createdAt: "2026-02-01T08:00", sortOrder: 3 });
    const legacy = makeTask({ id: 7, createdAt: null, sortOrder: 1 });

    expect(sortParentCandidates([later, earlier, legacy]).map((task) => task.id)).toEqual([
      7, 6, 5,
    ]);
  });
});

describe("buildGroups", () => {
  const modules = [
    makeModule(10, "前端", [{ id: 100, name: "样式" }]),
    makeModule(20, "后端"),
  ];

  it("puts the default group before modules; groups by submodule, module, default; subtasks get no group of their own", () => {
    const groups = buildGroups(
      [
        makeTask({ id: 1, moduleId: 10, submoduleId: 100 }),
        makeTask({ id: 2, moduleId: 10 }),
        makeTask({ id: 3 }),
        // A subtask follows its parent even when it carries classification fields; it takes no group.
        makeTask({ id: 4, moduleId: 10, submoduleId: 100, parentTaskId: 1 }),
      ],
      modules,
    );

    expect(groups.map((group) => [group.kind, group.id, group.label])).toEqual([
      ["unclassified", null, null],
      ["module", 10, "前端"],
      ["module", 20, "后端"],
    ]);
    expect(groups[0].tasks.map((task) => task.id)).toEqual([3]);
    expect(groups[1].tasks.map((task) => task.id)).toEqual([2]);
    expect(groups[1].children[0].tasks.map((task) => task.id)).toEqual([1]);
    expect(groups[2].tasks).toEqual([]);
    expect(groups[2].children).toEqual([]);
  });

  it("keeps user-created modules and submodules even when empty, and hides an empty default group", () => {
    const groups = buildGroups([], [makeModule(10, "空模块", [{ id: 100, name: "空子模块" }])]);

    expect(groups).toHaveLength(1);
    expect(groups[0].tasks).toEqual([]);
    expect(groups[0].children.map((group) => group.label)).toEqual(["空子模块"]);
  });

  it("reuses sortTasks ordering inside a group", () => {
    const groups = buildGroups(
      [
        makeTask({ id: 1, moduleId: 10, priority: "low" }),
        makeTask({ id: 2, moduleId: 10, priority: "high" }),
        makeTask({ id: 3, moduleId: 10, priority: "mid" }),
      ],
      [makeModule(10, "前端")],
    );

    expect(groups[0].tasks.map((task) => task.id)).toEqual([2, 3, 1]);
  });
});

describe("countGroup / countGroups", () => {
  it("counts parent tasks only, adds child groups recursively, and only counts done as done", () => {
    const groups = buildGroups(
      [
        makeTask({ id: 1, moduleId: 1, status: "done" }),
        makeTask({ id: 2, moduleId: 1 }),
        makeTask({ id: 3, moduleId: 1, submoduleId: 11, status: "done" }),
        makeTask({ id: 4, moduleId: 1, submoduleId: 11, parentTaskId: 3 }),
      ],
      [makeModule(1, "模块", [{ id: 11, name: "子模块" }])],
    );

    expect(countGroup(groups[0])).toEqual({ done: 2, total: 3 });
    expect(countGroups(groups)).toEqual({ done: 2, total: 3 });
  });
});

describe("selectScope", () => {
  const groups = buildGroups(
    [
      makeTask({ id: 1, moduleId: 10, submoduleId: 100 }),
      makeTask({ id: 2, moduleId: 20 }),
      makeTask({ id: 3 }),
    ],
    [makeModule(10, "前端", [{ id: 100, name: "样式" }]), makeModule(20, "后端")],
  );

  it("returns the whole tree for a project", () => {
    expect(selectScope(groups, { kind: "project" })).toHaveLength(3);
  });

  it("returns only the module for a module scope and only the default group for the default scope", () => {
    expect(selectScope(groups, { kind: "module", id: 20 }).map((group) => group.id)).toEqual([20]);
    expect(
      selectScope(groups, { kind: "unclassified" }).map((group) => group.kind),
    ).toEqual(["unclassified"]);
  });

  it("promotes a submodule to the root with depth zero and keeps its tasks", () => {
    const scoped = selectScope(groups, { kind: "submodule", id: 100 });

    expect(scoped).toHaveLength(1);
    expect(scoped[0].id).toBe(100);
    expect(scoped[0].depth).toBe(0);
    expect(scoped[0].tasks.map((task) => task.id)).toEqual([1]);
  });

  it("returns an empty array when the submodule is not found", () => {
    expect(selectScope(groups, { kind: "submodule", id: 999 })).toEqual([]);
  });
});

describe("resolveTarget", () => {
  const groups = buildGroups([], [makeModule(10, "前端", [{ id: 100, name: "样式" }])]);

  it("maps project and default scopes to the default position", () => {
    expect(resolveTarget(groups, { kind: "project" })).toEqual({
      moduleId: null,
      submoduleId: null,
    });
    expect(resolveTarget(groups, { kind: "unclassified" })).toEqual({
      moduleId: null,
      submoduleId: null,
    });
  });

  it("a module scope carries only moduleId", () => {
    expect(resolveTarget(groups, { kind: "module", id: 10 })).toEqual({
      moduleId: 10,
      submoduleId: null,
    });
  });

  it("a submodule scope must carry both moduleId and submoduleId", () => {
    expect(resolveTarget(groups, { kind: "submodule", id: 100 })).toEqual({
      moduleId: 10,
      submoduleId: 100,
    });
  });

  it("falls back to default when the owning submodule is not found", () => {
    expect(resolveTarget(groups, { kind: "submodule", id: 999 })).toEqual({
      moduleId: null,
      submoduleId: null,
    });
  });
});

describe("taskMatchesFilters / filterTasks", () => {
  it("combines status and priority with AND and values within a set with OR", () => {
    const todoHigh = makeTask({ id: 1, status: "todo", priority: "high" });
    const todoLow = makeTask({ id: 2, status: "todo", priority: "low" });
    const doneHigh = makeTask({ id: 3, status: "done", priority: "high" });
    const filters = {
      statuses: new Set<Status>(["todo"]),
      priorities: new Set<Priority>(["high", "mid"]),
    };

    expect(taskMatchesFilters(todoHigh, filters)).toBe(true);
    expect(taskMatchesFilters(todoLow, filters)).toBe(false);
    expect(taskMatchesFilters(doneHigh, filters)).toBe(false);
    expect(filterTasks([todoHigh, todoLow, doneHigh], filters)).toEqual([todoHigh]);
  });
});

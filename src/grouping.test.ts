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
  it("按优先级、再按创建时间升序排序，且不修改原数组", () => {
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

  it("已完成整组排在最后，组内仍按优先级 → 创建时间", () => {
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

  it("创建时间缺失时视为最早；时间相同再按添加顺序与 id", () => {
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

describe("sortTasks（列排序）", () => {
  it("按优先级升序 / 降序，且不再把已完成单独后置", () => {
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

  it("按开始时间升降序，未记录时间的任务始终排在最后", () => {
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

  it("排序值相同时回退到创建时间与添加顺序", () => {
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
  it("未完成 / 进行中在前，已完成在后；同状态内按创建时间升序", () => {
    const doneEarly = makeTask({ id: 1, status: "done", createdAt: "2026-01-01T08:00", sortOrder: 1 });
    const todoLate = makeTask({ id: 2, status: "todo", createdAt: "2026-03-01T08:00", sortOrder: 2 });
    const doingMid = makeTask({ id: 3, status: "doing", createdAt: "2026-02-01T08:00", sortOrder: 3 });
    const doneLate = makeTask({ id: 4, status: "done", createdAt: "2026-04-01T08:00", sortOrder: 4 });
    const tasks = [doneEarly, todoLate, doingMid, doneLate];
    const before = [...tasks];

    expect(sortParentCandidates(tasks).map((task) => task.id)).toEqual([3, 2, 1, 4]);
    expect(tasks).toEqual(before);
  });

  it("创建时间相同（或未记录）时按添加顺序，再按 id", () => {
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

  it("默认分组排在模块之前；按子模块、模块、默认归组，子任务不单独成组", () => {
    const groups = buildGroups(
      [
        makeTask({ id: 1, moduleId: 10, submoduleId: 100 }),
        makeTask({ id: 2, moduleId: 10 }),
        makeTask({ id: 3 }),
        // 子任务即使带着分类字段，也跟着父任务走，不占组。
        makeTask({ id: 4, moduleId: 10, submoduleId: 100, parentTaskId: 1 }),
      ],
      modules,
    );

    expect(groups.map((group) => [group.kind, group.id, group.label])).toEqual([
      ["unclassified", null, "默认"],
      ["module", 10, "前端"],
      ["module", 20, "后端"],
    ]);
    expect(groups[0].tasks.map((task) => task.id)).toEqual([3]);
    expect(groups[1].tasks.map((task) => task.id)).toEqual([2]);
    expect(groups[1].children[0].tasks.map((task) => task.id)).toEqual([1]);
    expect(groups[2].tasks).toEqual([]);
    expect(groups[2].children).toEqual([]);
  });

  it("用户建的模块和子模块即使为空也保留，空默认分组不出现", () => {
    const groups = buildGroups([], [makeModule(10, "空模块", [{ id: 100, name: "空子模块" }])]);

    expect(groups).toHaveLength(1);
    expect(groups[0].tasks).toEqual([]);
    expect(groups[0].children.map((group) => group.label)).toEqual(["空子模块"]);
  });

  it("组内任务复用 sortTasks 的排序", () => {
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
  it("只数父任务，子分组递归累加，done 只数已完成", () => {
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

  it("项目返回整棵树", () => {
    expect(selectScope(groups, { kind: "project" })).toHaveLength(3);
  });

  it("模块范围只返回该模块，默认范围只返回默认组", () => {
    expect(selectScope(groups, { kind: "module", id: 20 }).map((group) => group.id)).toEqual([20]);
    expect(
      selectScope(groups, { kind: "unclassified" }).map((group) => group.kind),
    ).toEqual(["unclassified"]);
  });

  it("子模块被提升为根，depth 归零，任务保留", () => {
    const scoped = selectScope(groups, { kind: "submodule", id: 100 });

    expect(scoped).toHaveLength(1);
    expect(scoped[0].id).toBe(100);
    expect(scoped[0].depth).toBe(0);
    expect(scoped[0].tasks.map((task) => task.id)).toEqual([1]);
  });

  it("找不到子模块时返回空数组", () => {
    expect(selectScope(groups, { kind: "submodule", id: 999 })).toEqual([]);
  });
});

describe("resolveTarget", () => {
  const groups = buildGroups([], [makeModule(10, "前端", [{ id: 100, name: "样式" }])]);

  it("项目与默认分组落到默认位置", () => {
    expect(resolveTarget(groups, { kind: "project" })).toEqual({
      moduleId: null,
      submoduleId: null,
    });
    expect(resolveTarget(groups, { kind: "unclassified" })).toEqual({
      moduleId: null,
      submoduleId: null,
    });
  });

  it("模块范围只带 moduleId", () => {
    expect(resolveTarget(groups, { kind: "module", id: 10 })).toEqual({
      moduleId: 10,
      submoduleId: null,
    });
  });

  it("子模块范围必须同时带 moduleId 与 submoduleId", () => {
    expect(resolveTarget(groups, { kind: "submodule", id: 100 })).toEqual({
      moduleId: 10,
      submoduleId: 100,
    });
  });

  it("找不到归属的子模块回退到默认", () => {
    expect(resolveTarget(groups, { kind: "submodule", id: 999 })).toEqual({
      moduleId: null,
      submoduleId: null,
    });
  });
});

describe("taskMatchesFilters / filterTasks", () => {
  it("状态与优先级是且关系，集合内是或关系", () => {
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

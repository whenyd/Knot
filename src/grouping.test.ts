import { describe, expect, it } from "vitest";

import type { Module, Priority, Status, Task } from "./api";
import {
  buildGroups,
  countGroup,
  countGroups,
  filterTasks,
  resolveTarget,
  selectScope,
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
  it("按优先级、再按添加顺序排序，且不修改原数组", () => {
    const low = makeTask({ id: 1, priority: "low", sortOrder: 1 });
    const highLater = makeTask({ id: 2, priority: "high", sortOrder: 9 });
    const mid = makeTask({ id: 3, priority: "mid", sortOrder: 2 });
    const highEarlier = makeTask({ id: 4, priority: "high", sortOrder: 3 });
    const tasks = [low, highLater, mid, highEarlier];
    const before = [...tasks];

    expect(sortTasks(tasks).map((task) => task.id)).toEqual([4, 2, 3, 1]);
    expect(tasks).toEqual(before);
  });
});

describe("buildGroups", () => {
  const modules = [
    makeModule(10, "前端", [{ id: 100, name: "样式" }]),
    makeModule(20, "后端"),
  ];

  it("按子模块、模块、默认归组；子任务不单独成组", () => {
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
      ["module", 10, "前端"],
      ["module", 20, "后端"],
      ["unclassified", null, "默认"],
    ]);
    expect(groups[0].tasks.map((task) => task.id)).toEqual([2]);
    expect(groups[0].children[0].tasks.map((task) => task.id)).toEqual([1]);
    expect(groups[1].tasks).toEqual([]);
    expect(groups[1].children).toEqual([]);
    expect(groups[2].tasks.map((task) => task.id)).toEqual([3]);
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

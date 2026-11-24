import { describe, expect, it } from "vitest";

import catalogJson from "../src-tauri/backend-keys.json";
import indexHtml from "../index.html?raw";
import { describe as describeError } from "./errors";
import { backendTables, translateBackendMessage } from "./i18n/backend";
import { formatSchedule } from "./i18n/date";
import { hasMessage, setLanguageForTest, tp } from "./i18n";
import { en } from "./i18n/messages.en";
import { zh, type MessageKey } from "./i18n/messages.zh";

function placeholders(template: string): string[] {
  return [...template.matchAll(/\{(\w+)\}/g)].map((match) => match[1]).sort();
}

const catalog = catalogJson as {
  keys: Record<string, { params: string[]; plural?: boolean }>;
};

describe("UI dictionaries", () => {
  it("English covers exactly the Chinese keys", () => {
    expect(Object.keys(en).sort()).toEqual(Object.keys(zh).sort());
  });

  it("both languages use the same placeholders for every key", () => {
    for (const key of Object.keys(zh) as MessageKey[]) {
      expect(placeholders(en[key]), `placeholders differ for ${key}`).toEqual(
        placeholders(zh[key]),
      );
    }
  });

  it("every data-i18n key in index.html exists", () => {
    const keys = [
      ...indexHtml.matchAll(/data-i18n(?:-[a-z-]+)?="([^"]+)"/g),
    ].map((match) => match[1]);
    expect(keys.length).toBeGreaterThan(0);
    for (const key of keys) {
      expect(hasMessage(key), `missing key ${key}`).toBe(true);
    }
  });

  it("plural helpers pick the language-specific form", () => {
    setLanguageForTest("en");
    expect(tp("task.subtaskDoneCount", 1, { done: 0, total: 1 })).toBe("0 of 1 subtask done");
    expect(tp("task.subtaskDoneCount", 2, { done: 1, total: 2 })).toBe(
      "1 of 2 subtasks done",
    );
    setLanguageForTest("zh");
    expect(tp("task.subtaskDoneCount", 1, { done: 0, total: 1 })).toBe(
      "子任务已完成 0 个，共 1 个",
    );
  });
});

describe("backend message catalog", () => {
  it("Rust keys, both tables and the catalog match", () => {
    for (const [key, meta] of Object.entries(catalog.keys)) {
      for (const language of ["zh", "en"] as const) {
        const table = backendTables[language];
        const template = table[key] ?? table[`${key}.other`];
        expect(template, `${language} is missing ${key}`).toBeDefined();
        if (meta.plural) {
          expect(table[`${key}.one`], `${language} is missing ${key}.one`).toBeDefined();
          expect(table[`${key}.other`], `${language} is missing ${key}.other`).toBeDefined();
        }
        const used = new Set(placeholders(template));
        for (const param of meta.params) {
          expect(used.has(param), `${key} does not use {${param}}`).toBe(true);
        }
        expect([...used].sort()).toEqual([...meta.params].sort());
      }
    }
  });

  it("tables contain no keys outside the catalog", () => {
    const catalogKeys = new Set(Object.keys(catalog.keys));
    for (const [language, table] of Object.entries(backendTables)) {
      for (const key of Object.keys(table)) {
        const variant = /^(.*)\.(one|other)$/.exec(key);
        const base = variant ? variant[1] : key;
        expect(catalogKeys.has(base), `${language} has extra key ${key}`).toBe(true);
      }
    }
  });

  it("translates structured messages, entity tokens and plurals", () => {
    setLanguageForTest("zh");
    expect(
      translateBackendMessage({ key: "resource.notFound", params: { entity: "task", id: 3 } }),
    ).toBe("任务 3 不存在");
    expect(
      translateBackendMessage({ key: "confirm.cascadeDone", params: { count: 1 } }),
    ).toContain("1 个未完成子任务");

    setLanguageForTest("en");
    expect(
      translateBackendMessage({ key: "resource.notFound", params: { entity: "task", id: 3 } }),
    ).toBe("Task 3 was not found.");
    expect(
      translateBackendMessage({ key: "confirm.cascadeDone", params: { count: 1 } }),
    ).toContain("1 unfinished subtask.");
    expect(
      translateBackendMessage({ key: "confirm.cascadeDone", params: { count: 2 } }),
    ).toContain("2 unfinished subtasks.");
    expect(translateBackendMessage({ key: "unknown.key" })).toContain("unknown.key");
  });
});

describe("date formatting", () => {
  it("uses a localized display format without changing stored values", () => {
    setLanguageForTest("zh");
    expect(formatSchedule("2026-11-04T09:00")).toBe("2026/11/04 09:00");
    expect(formatSchedule("2026-11-04T00:00")).toBe("2026/11/04 00:00");

    setLanguageForTest("en");
    expect(formatSchedule("2026-11-04T09:00")).toBe("Nov 4, 2026, 9:00 AM");

    expect(formatSchedule(null)).toBe("—");
    expect(formatSchedule("not-a-date")).toBe("not-a-date");
  });
});

describe("describe", () => {
  it("handles structured messages, strings, Errors and unknown values", () => {
    setLanguageForTest("en");
    expect(
      describeError({ key: "notice.parentAutoDone" }),
    ).toBe("The parent task was marked as done automatically");
    expect(describeError("boom")).toBe("Something went wrong: boom");
    expect(describeError(new Error("bad"))).toBe("Something went wrong: bad");
    expect(describeError(42)).toBe("Unknown error");

    setLanguageForTest("zh");
    expect(describeError("boom")).toBe("发生错误：boom");
    expect(describeError({ key: "notice.parentAutoDone" })).toBe("父任务已自动标记为已完成");
  });
});

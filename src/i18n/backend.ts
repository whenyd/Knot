/**
 * Translations for structured backend messages.
 *
 * `src-tauri/backend-keys.json` is the contract; `i18n.test.ts` checks that
 * both tables cover exactly those keys and use the same placeholders.
 */
import type { BackendMessage, MessageParam } from "../api";
import { getLanguage, interpolate, LOCALES, t, type Language } from "./index";

type Table = Record<string, string>;

const ENTITY_NAMES: Record<Language, Record<string, string>> = {
  zh: { project: "项目", module: "模块", submodule: "子模块", task: "任务" },
  en: { project: "Project", module: "Module", submodule: "Submodule", task: "Task" },
};

const FIELD_NAMES: Record<Language, Record<string, string>> = {
  zh: { project: "项目名", module: "模块名", submodule: "子模块名", task: "任务文字" },
  en: {
    project: "Project name",
    module: "Module name",
    submodule: "Submodule name",
    task: "Task text",
  },
};

const NOUN_NAMES: Record<Language, Record<string, string>> = {
  zh: { module: "模块", submodule: "子模块" },
  en: { module: "module", submodule: "submodule" },
};

const zhBackend: Table = {
  "validation.nameRequired": "{field}不能为空",
  "validation.unknownStatus": "未知的状态值 {value}",
  "validation.unknownPriority": "未知的优先级值 {value}",
  "validation.moduleOrderSameProject": "只能在同一项目内调整模块顺序",
  "validation.subtaskCannotNest": "子任务不能再挂子任务",
  "validation.subtaskContainerMismatch": "子任务必须与父任务在同一个容器",
  "validation.subtaskPriorityAboveParent": "子任务优先级不能高于父任务",
  "validation.subtaskCannotMove": "子任务跟随父任务，不能单独移动",
  "validation.onlyTopLevelCanDemote": "只有顶层任务可以降为子任务",
  "validation.hasChildrenCannotDemote": "这个任务还有子任务，不能直接降级",
  "validation.cannotDemoteToSelf": "不能把任务降为自己的子任务",
  "validation.subtaskParentContainerMismatch": "子任务必须和父任务在同一个容器",
  "validation.alreadyTopLevel": "这个任务已经是顶层任务",
  "validation.todoHasNoModules": "「我的待办」里的任务不属于任何模块",
  "validation.submoduleNeedsModule": "子模块必须附属于一个模块",
  "validation.moduleNotInProject": "模块 {module} 不属于项目 {project}",
  "validation.submoduleNotInModule": "子模块 {submodule} 不属于模块 {module}",
  "resource.notFound": "{entity} {id} 不存在",
  "confirm.cascadeDone.one":
    "这个任务还有 {count} 个未完成子任务，确认后会一并标记为已完成。",
  "confirm.cascadeDone.other":
    "这个任务还有 {count} 个未完成子任务，确认后会一并标记为已完成。",
  "notice.parentAutoDone": "父任务已自动标记为已完成",
  "notice.parentRestoredDoing": "父任务已恢复为进行中",
  "notice.parentAutoDoing": "父任务已自动标记为进行中",
  "notice.parentRestoredTodo": "父任务已恢复为未开始",
  "notice.parentAutoTodo": "父任务已自动标记为未开始",
  "db.operationFailed": "数据库操作失败：{detail}",
  "db.resetPrepareFailed": "无法准备重置数据库：{detail}",
  "db.reopenFailed": "重新打开数据库也失败：{detail}",
  "db.dataDirFailed": "无法定位应用数据目录：{detail}",
  "db.createDirFailed": "无法创建目录 {path}：{detail}",
  "db.openFailed": "无法打开数据库 {path}：{detail}",
  "db.foreignKeysFailed": "无法开启外键约束：{detail}",
  "db.backupCopyFailed": "迁移前备份失败，已停止迁移：无法把 {from} 复制到 {to}：{detail}",
  "db.deleteFailed": "无法删除数据库 {path}：{detail}",
  "startup.readVersionFailed": "无法读取 user_version：{detail}",
  "startup.newerDatabase":
    "数据库版本 {current} 高于本程序支持的 {target}，拒绝启动以免损坏数据",
  "startup.disableForeignKeysFailed": "无法临时关闭外键约束：{detail}",
  "startup.enableForeignKeysFailed": "无法恢复外键约束：{detail}",
  "startup.beginTransactionFailed": "无法开启迁移事务：{detail}",
  "startup.migrationFailed": "迁移到版本 {version} 失败：{detail}",
  "startup.writeVersionFailed": "迁移到版本 {version} 后写 user_version 失败：{detail}",
  "startup.commitFailed": "提交迁移事务失败：{detail}",
  "startup.foreignKeyCheckFailed": "无法执行外键校验：{detail}",
  "startup.foreignKeyCheckReadFailed": "无法读取外键校验结果：{detail}",
  "startup.foreignKeyViolations.one": "迁移校验失败（已回滚）：{count} 条悬空引用",
  "startup.foreignKeyViolations.other": "迁移校验失败（已回滚）：{count} 条悬空引用",
};

const enBackend: Table = {
  "validation.nameRequired": "{field} cannot be empty.",
  "validation.unknownStatus": "Unknown status value {value}.",
  "validation.unknownPriority": "Unknown priority value {value}.",
  "validation.moduleOrderSameProject": "Modules can only be reordered within the same project.",
  "validation.subtaskCannotNest": "A subtask cannot have subtasks.",
  "validation.subtaskContainerMismatch":
    "A subtask must stay in the same container as its parent task.",
  "validation.subtaskPriorityAboveParent":
    "A subtask cannot have a higher priority than its parent task.",
  "validation.subtaskCannotMove": "A subtask follows its parent and cannot be moved on its own.",
  "validation.onlyTopLevelCanDemote": "Only top-level tasks can be demoted to subtasks.",
  "validation.hasChildrenCannotDemote":
    "This task still has subtasks and cannot be demoted directly.",
  "validation.cannotDemoteToSelf": "A task cannot be demoted under itself.",
  "validation.subtaskParentContainerMismatch":
    "A subtask must stay in the same container as its parent task.",
  "validation.alreadyTopLevel": "This task is already top-level.",
  "validation.todoHasNoModules": "Tasks in My Tasks cannot belong to a module.",
  "validation.submoduleNeedsModule": "A submodule must belong to a module.",
  "validation.moduleNotInProject": "Module {module} does not belong to project {project}.",
  "validation.submoduleNotInModule": "Submodule {submodule} does not belong to module {module}.",
  "resource.notFound": "{entity} {id} was not found.",
  "confirm.cascadeDone.one":
    "This task has {count} unfinished subtask. Confirming will mark it as done too.",
  "confirm.cascadeDone.other":
    "This task has {count} unfinished subtasks. Confirming will mark them as done too.",
  "notice.parentAutoDone": "The parent task was marked as done automatically",
  "notice.parentRestoredDoing": "The parent task was restored to doing",
  "notice.parentAutoDoing": "The parent task was marked as doing automatically",
  "notice.parentRestoredTodo": "The parent task was restored to todo",
  "notice.parentAutoTodo": "The parent task was marked as todo automatically",
  "db.operationFailed": "Database operation failed: {detail}",
  "db.resetPrepareFailed": "Could not prepare the database reset: {detail}",
  "db.reopenFailed": "Reopening the database also failed: {detail}",
  "db.dataDirFailed": "Could not locate the application data directory: {detail}",
  "db.createDirFailed": "Could not create directory {path}: {detail}",
  "db.openFailed": "Could not open database {path}: {detail}",
  "db.foreignKeysFailed": "Could not enable foreign key checks: {detail}",
  "db.backupCopyFailed":
    "The pre-migration backup failed, so migration stopped: could not copy {from} to {to}: {detail}",
  "db.deleteFailed": "Could not delete database {path}: {detail}",
  "startup.readVersionFailed": "Could not read user_version: {detail}",
  "startup.newerDatabase":
    "Database version {current} is newer than the supported {target}; refusing to start to avoid corrupting it.",
  "startup.disableForeignKeysFailed": "Could not disable foreign key checks temporarily: {detail}",
  "startup.enableForeignKeysFailed": "Could not restore foreign key checks: {detail}",
  "startup.beginTransactionFailed": "Could not begin the migration transaction: {detail}",
  "startup.migrationFailed": "Migration to version {version} failed: {detail}",
  "startup.writeVersionFailed":
    "Writing user_version after migrating to version {version} failed: {detail}",
  "startup.commitFailed": "Could not commit the migration transaction: {detail}",
  "startup.foreignKeyCheckFailed": "Could not run the foreign key check: {detail}",
  "startup.foreignKeyCheckReadFailed": "Could not read the foreign key check result: {detail}",
  "startup.foreignKeyViolations.one":
    "Migration validation failed (rolled back): {count} dangling reference.",
  "startup.foreignKeyViolations.other":
    "Migration validation failed (rolled back): {count} dangling references.",
};

const tables: Record<Language, Table> = { zh: zhBackend, en: enBackend };

function isBackendMessage(value: unknown): value is BackendMessage {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as { key?: unknown }).key === "string"
  );
}

function translateParam(name: string, value: MessageParam): string | number {
  if (typeof value === "number") return value;
  if (isBackendMessage(value)) return translateBackendMessage(value);

  const language = getLanguage();
  if (name === "entity") return ENTITY_NAMES[language][value] ?? value;
  if (name === "field") return FIELD_NAMES[language][value] ?? value;
  if (name === "noun") return NOUN_NAMES[language][value] ?? value;
  return value;
}

/** Translate one structured backend message into the active language. */
export function translateBackendMessage(message: BackendMessage): string {
  const language = getLanguage();
  const table = tables[language];
  let template = table[message.key];

  if (template === undefined) {
    const count = Number(message.params?.count ?? 0);
    const category = new Intl.PluralRules(LOCALES[language]).select(count);
    const suffix = category === "one" ? "one" : "other";
    template = table[`${message.key}.${suffix}`] ?? table[`${message.key}.other`];
  }

  if (template === undefined) {
    return t("error.unknownBackendMessage", { key: message.key });
  }

  const params: Record<string, string | number> = {};
  for (const [name, value] of Object.entries(message.params ?? {})) {
    params[name] = translateParam(name, value);
  }
  return interpolate(template, params);
}

/** Exposed for the catalog test. */
export const backendTables = tables;

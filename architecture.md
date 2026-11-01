---
id: architecture
type: architecture-design
title: 架构 — 本机 SQLite + Tauri 命令边界
status: draft
parent: goal-and-requirements
---

# 架构

一条约束贯穿全篇：**数据只有一份，在用户的本机文件里**。所以凡是"可能悄悄把数据改坏"的决定（迁移、级联、联动）都倾向保守：先备份、先确认、只写前进。

## 分层

```
前端 (Vite + TS, 原生 DOM)
   │  只发命令、只渲染；不认识任何聚合规则
   ▼
Tauri 命令层 (commands.rs)      —— 薄壳：参数校验、开事务、调 domain
   ▼
领域层 (domain.rs)              —— 唯一写入口：状态联动、级联、排序号分配
   ▼
存储层 (db.rs + migrations.rs)  —— rusqlite、连接、user_version 迁移
   ▼
%APPDATA%/todolist/tasks.sqlite
```

规则只允许住在 `domain.rs`。前端不重算、SQL 触发器不重复表达——否则同一套规则会有两个实现，必然漂移。

## 数据模型

```sql
CREATE TABLE project (
  id          INTEGER PRIMARY KEY,
  name        TEXT    NOT NULL,
  sort_order  INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE module (
  id          INTEGER PRIMARY KEY,
  project_id  INTEGER NOT NULL REFERENCES project(id) ON DELETE CASCADE,
  name        TEXT    NOT NULL,
  sort_order  INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE submodule (
  id          INTEGER PRIMARY KEY,
  module_id   INTEGER NOT NULL REFERENCES module(id) ON DELETE CASCADE,
  name        TEXT    NOT NULL,
  sort_order  INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE task (
  id             INTEGER PRIMARY KEY,
  project_id     INTEGER REFERENCES project(id) ON DELETE CASCADE,   -- NULL = 「待办」
  module_id      INTEGER REFERENCES module(id) ON DELETE SET NULL,
  submodule_id   INTEGER REFERENCES submodule(id) ON DELETE SET NULL,
  parent_task_id INTEGER REFERENCES task(id) ON DELETE CASCADE,      -- NULL = 顶层任务
  title          TEXT    NOT NULL,
  status         TEXT    NOT NULL DEFAULT 'todo' CHECK (status IN ('todo','doing','done')),
  priority       TEXT    NOT NULL DEFAULT 'mid'  CHECK (priority IN ('high','mid','low')),
  note           TEXT    NOT NULL DEFAULT '',
  sort_order     INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_task_project ON task(project_id);
CREATE INDEX idx_task_module  ON task(module_id);
CREATE INDEX idx_task_parent  ON task(parent_task_id);
```

几个刻意的选择：

- **「待办」不是一行项目记录**，而是 `project_id IS NULL`。省掉一条"永远存在的系统记录"，也让"项目列表"查询天然干净。
- **子任务不存 `module_id`/`submodule_id`**（恒为 NULL）。子任务跟随父任务、不能单独移动，所以归属的唯一真相在父任务上；冗余一份只会漂移。
- **状态与优先级存文本而非整数**，用 `CHECK` 约束。库文件是给人看的，`'doing'` 比 `1` 好读，值集合也由 `CHECK` 兜住。
- **"只允许一层子任务"无法用 `CHECK` 表达**（SQLite 的 CHECK 不支持子查询），由 `domain.rs` 在创建时保证。
- 外键级联只在"删父带走子"上使用；`module`/`submodule` 被删时任务的 `ON DELETE SET NULL` 让它自然回到"未分类"，不再单独写清理逻辑。

## 迁移策略

- **版本号**：`PRAGMA user_version`，`MIGRATIONS: &[&str]` 从第 1 条起顺序执行；`user_version` 即"已应用到第几条"。
- **时机**：打开连接后、任何读写之前，整段包在**单个事务**里。失败则回滚、拒绝启动，并在界面上给出备份文件路径。
- **只写前进**：不写 down 迁移。down 长期没人执行，是假的安全感；回滚靠下面的备份。
- **迁移前自动备份**：库文件复制为 `tasks.sqlite.bak-v<旧版本>-<时间戳>`，只保留最近 5 份（更早的删除）。
- **外键开关**：每个连接显式 `PRAGMA foreign_keys = ON`（SQLite 默认关闭）。迁移事务内先关（重建表时外键会挡路），提交后跑一次 `PRAGMA foreign_key_check`，通过再打开。
- **开发期重置**：仅 dev build 提供"删库重建"入口，正式版不暴露。

## 状态联动（父子）

**聚合是事件触发的推导，不是恒成立的不变式。** 这是有意的：允许用户把父任务手动按下去，代价是可能出现“父未完成、子全完成”，直到下一次子任务变动才纠正。

触发聚合的三个事件（都在同一个事务内完成）：

1. 子任务状态变化
2. 子任务新增
3. 子任务删除

聚合动作：子任务全为 `done` → 父置 `done`；否则且父当前为 `done` → 父置 `doing`。父任务被自动改动时，返回一条 notice，前端显示几秒后消失的轻提示。

| 事件 | 父任务 | 子任务 | 用户感知 |
|---|---|---|---|
| 手动把父设为 `done`，子任务全完成 | → done | 不动 | 无 |
| 手动把父设为 `done`，有未完成子任务 | 先确认 → done | 一并 done | 弹窗告知影响条数 |
| 子任务变 `done` 且是最后一个 | → done | 不动 | 轻提示 |
| 子任务退回 / 新增未完成子任务 | → doing | 不动 | 轻提示 |
| 删除最后一个未完成子任务 | → done | 不动 | 轻提示 |
| 手动把父从 `done` 退回 | → 用户选的状态 | **不动** | 无（可能暂时不一致） |
| 删除父任务 | 删 | 一并删 | 弹窗告知条数 |

## 命令清单

所有写操作都返回受影响的行/节点，前端据此全量重取当前容器（数据量小，不做增量 patch）。“先确认再写”统一成 `plan_*` / `apply_*` 两段，前端不重算规则：

```
// 读
list_projects()                       -> [{ id, name, counts, is_todo }]   // 首项是虚拟的「待办」
list_modules(project_id)              -> [{ id, name, submodules: [...] }]
list_tasks(container)                 -> [task]   // container = { kind: 'todo' } | { kind: 'project', id }

// 写 · 任务
create_task({ container, module_id?, submodule_id?, parent_task_id?, title })
rename_task(id, title)                              // 就地编辑；trim 后为空则拒绝
set_note(id, note)
set_priority(id, priority)
move_task(id, { container, module_id?, submodule_id? })   // 同容器或跨容器

// 写 · 状态（两段式）
plan_set_status(id, status)           -> { affects: [task], needs_confirm, reason }
apply_set_status(id, status, cascade: bool) -> { changed: [task], notices: [string] }

// 写 · 结构（删除同样两段式，先拿数量再确认）
create_project(name) / rename_project(id, name) / plan_delete_project(id) -> { task_count } / delete_project(id)
create_module(project_id, name) / rename_module(id, name) / plan_delete_node(id) -> { task_count } / delete_node(id)
create_submodule(module_id, name) / rename_submodule(id, name)

// 删除任务
plan_delete_task(id) -> { task_count }   // 含子任务
delete_task(id)
```

## 前端结构

- **视图状态**：`container`（待办 / 某个项目）、`scope`（整个项目 / 某模块 / 某子模块 / 未分类）、`statusFilter: Set`、`priorityFilter: Set`、`expanded: Set<nodeId>`。
- **数据管线**（单一方向，不双向绑定）：`list_tasks` → 按 scope 过滤 → 按两个筛选集过滤 → 按 module/submodule 建树 → 分组按 `sort_order`、组内任务按 `priority` 再 `sort_order` 排序 → 渲染。
- **交互归属**：状态格点击只调 `plan_set_status`；`needs_confirm` 为真就弹窗；`apply_set_status` 回来后重取。就地编辑（双击）走 `rename_task`，空值前端还原、后端也拒绝。
- **轻提示**：表格底部一条队列，几秒后自动消失，不阻塞任何操作。

## 边界情况

- 空项目 / 空分组：显示空状态文案，不渲染空分组行。
- 「待办」视图：没有模块树、没有分组行，表格直接平铺（与目标文档一致）。
- 「未分类」：项目内 `module_id IS NULL` 的任务聚成一个固定分组行，排在模块分组之后。
- 编辑任务文字为空白：还原原文，不写库。
- 迁移失败：拒绝启动并显示备份路径，绝不带着半迁移的库继续跑。

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
  project_id     INTEGER REFERENCES project(id) ON DELETE CASCADE,    -- NULL = 「待办」
  module_id      INTEGER REFERENCES module(id) ON DELETE CASCADE,
  submodule_id   INTEGER REFERENCES submodule(id) ON DELETE CASCADE,
  parent_task_id INTEGER REFERENCES task(id) ON DELETE CASCADE,       -- NULL = 顶层任务
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
- 外键级联一律是"删父带走子"：删项目带走它的模块与任务，删模块带走它的子模块与任务，删任务带走它的子任务。**删除模块时组内的任务是真的被删掉，不是"退回未分类"**（两者都是数据，但只有一个是可恢复的，所以删除一律走 `plan_*` 先告知条数）。

> 注意：v1 迁移发布时这三列写的是 `ON DELETE SET NULL`（“退回未分类”）。已发布的迁移条目不能改，所以 v2 用"新建表 → 拷贝 → 删旧表 → 改名"把 `module_id`/`submodule_id` 改成 `CASCADE`（SQLite 不支持 ALTER 外键）。

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

聚合动作：子任务全为 `done` → 父置 `done`；否则且父当前为 `done` → 父置 `doing`。规则按字面实现：零个子任务时 `all()` 为真，所以删除最后一个未完成子任务会让父任务变 `done`。只有父状态**真的被改写**时才返回一条 notice；前端按 FIFO 排队显示几秒后自动消失的轻提示。手动改父任务（包括 cascade 写子任务）不触发聚合、不产生提示；删父任务只向下级联，不向上聚合。

| 事件 | 父任务 | 子任务 | 用户感知 |
|---|---|---|---|
| 手动把父设为 `done`，子任务全完成 | → done | 不动 | 无 |
| 手动把父设为 `done`，有未完成子任务 | 先确认 → done | 一并 done | 弹窗告知影响条数 |
| 子任务变 `done` 且是最后一个 | → done | 不动 | 轻提示 |
| 子任务变 `done` 但还有未完成兄弟 | 不动 | 不动 | 无 |
| 子任务退回 / 新增未完成子任务 | → doing | 不动 | 轻提示 |
| 删除最后一个未完成子任务 | → done（零子任务也按全完成算） | 不动 | 轻提示 |
| 删除子任务但还有未完成兄弟 | 不动 | 不动 | 无 |
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
create_task({ container, module_id?, submodule_id?, parent_task_id?, title }) -> { task, notices }
rename_task(id, title)                              // 就地编辑；trim 后为空则拒绝
set_note(id, note)
set_priority(id, priority)
move_task(id, { container, module_id?, submodule_id? })   // 同容器或跨容器

// 写 · 状态（两段式）
plan_set_status(id, status)           -> { affects: [task], needs_confirm, reason }
apply_set_status(id, status, cascade: bool) -> { changed: [task], notices: [string] }

// 写 · 结构（删除同样两段式，先拿数量再确认）
create_project(name) / rename_project(id, name) / plan_delete_project(id) -> { task_count } / delete_project(id)
create_module(project_id, name) / rename_module(id, name)
create_submodule(module_id, name) / rename_submodule(id, name)

// 删除结构节点（模块和子模块的 id 各自独立，必须带 kind 区分）
plan_delete_node(kind, id) -> { task_count } / delete_node(kind, id)   // kind = 'module' | 'submodule'

// 删除任务
plan_delete_task(id) -> { task_count }   // 含子任务
delete_task(id) -> { notices }
```

删除的条数一律包含会被级联带走的子任务；`plan_*` 只读，前端拿 `task_count` 填确认框，用户确认后才调 `delete_*`。

**错误约定**：命令的 `Err` 一律是给人看的一句话（`String`），不用结构化错误对象——前端只做一件事：在底部轻提示里原样显示。需要分类处理的场景（`needs_confirm`、条数）走 `Ok` 里的字段，不走错误通道。

**字段名约定**：跨 IPC 的结构一律 camelCase（Rust 侧 `#[serde(rename_all = "camelCase")]`，枚举值小写），前端类型与之逐字对应。字段名归后端所有，前端不另行映射。`domain.rs` 里有几条单测专门盯着这个契约。

**归属参数归后端校验**：`create_task` 接受 `module_id`/`submodule_id`，由 `domain.rs` 验证「模块属于该项目」「子模块属于该模块」「「待办」的任务不能带模块」。前端只负责把当前范围翻译成这两个 id（`resolveTarget`），不代替后端判定合法性。

**子任务的归属规则**：给了 `parent_task_id` 时，后端拒绝「父任务已有父任务」（只允许一层）与「父任务不在当前容器」；子任务落库时 `module_id`/`submodule_id` 一律为 NULL（调用方传了也忽略），`project_id` 跟随父任务，排序号按该父任务下 `MAX+1` 单独排队。父任务的分类就是子任务的分类，不需要也不允许冗余一份。

**默认值归属**：`status`/`priority` 的默认值由 `domain.rs` 在 INSERT 时显式写出，schema 的 `DEFAULT` 只作兜底。同一条规则不在两处表达，改默认值只有一个地方要动。

## 前端结构

- **视图状态**：`container`（待办 / 某个项目）、`scope`（全部 / 某模块 / 某子模块 / 未分类）、`statusFilter: Set`、`priorityFilter: Set`、`collapsedNodes: Set<nodeId>`（左栏范围树收起的模块）。
- **一次渲染只建一棵树**：`buildGroups` 的结果同时给左栏范围树算计数、给表格做分组，`selectScope` 再把范围套上去——同一棵树不会被算两遍、更不会两份不一致。
- **两处折叠互不影响**：左栏范围树的收起（`collapsedNodes`）只管树的显示，表格分组行的收起（`collapsedGroups`）只管表格的行。一个是导航，一个是阅读密度。
- **数据管线**（单一方向，不双向绑定）：`list_tasks` → 按 scope 过滤 → 按两个筛选集过滤 → 按 module/submodule 建树 → 分组按 `sort_order`、组内任务按 `priority` 再 `sort_order` 排序 → 渲染。
- **交互归属**：状态格点击只调 `plan_set_status`；`needs_confirm` 为真就弹窗；`apply_set_status` 回来后重取。就地编辑（双击）走 `rename_task`，空值前端还原、后端也拒绝。
- **弹窗有两种形态，共用一个 modal**：`askConfirm`（告知影响后确认）与 `askText`（改名/新建时输入一个名字）。**同一时刻只能有一个弹窗**——新弹窗会把上一个按“取消”收场，否则两个 `await` 会同时悬着、抢同一批 DOM 监听，一次回车跑两个命令（已发生过）。输入框里空白名不关窗也不出库。
- **内联新建项目表单**是弹窗以外唯一的输入形态（左栏「＋ 新建项目」）。它只有一个收场出口 `resetProjectForm()`（隐藏 + 清空）：提交成功、点表单外（`document` 上的 `mousedown`，表单内不关）、Esc 都走它；打开时挂监听、关闭时摘掉，不留常驻监听。已输入的文字跟随表单一起丢弃——不保留草稿，也不计入状态。
- **菜单只有一份实现**：`src/menu.ts` 负责弹出、定位、关闭，三处共用——左栏项目/范围节点的右键菜单、任务行尾的「…」菜单、优先级的 `.menu` 浮窗。入口只给 `items` 和位置（指针点或元素锚点），菜单项的行为由调用方提供。带 `children` 的条目（目前只有「移动到…」）不内联展开，而是悬停/点击后在父行右侧弹出独立的 `.menu` 子面板；右侧放不下翻到左侧，父面板布局不重排。一次只保留一条面板路径：打开同级子面板会先关掉旧的；根面板滚动会关掉子面板。打开时先关掉上一个；点外面 / Esc / 滚屏 / 窗口失焦关闭；上下键在当前面板内循环，右键进子面板、左键退回父行，Esc 把焦点还给打开它的按钮。锚点定位在下方放不下时翻到上方，并夹在视口内。
- **删除的回落**：删除任务只是重取当前容器；删除模块/子模块时，如果当前 `scope` 正落在被删的子树里，回到项目「全部」；删除项目时，如果当前 `container` 正是它，回到「待办」。删除前都用 `plan_*` 的条数填确认框，不在前端重算级联范围。
- **子任务渲染**：`buildGroups` 继续跳过 `parentTaskId !== null` 的任务（它们不占分组行，分组计数只算父任务）；渲染时另建 `parentTaskId → tasks` 的索引，父任务行后按同一套排序（优先级 → `sort_order`）渲染缩进子行。父任务行默认展开，`collapsedTasks` 只记本次会话里收起的那几个；父任务标题前的折叠箭头与行内「＋」只出现在顶层任务上。
- **行内子任务输入**：`addingSubtaskFor` 同时只有一个父任务，输入草稿放在 `subtaskDraft`（状态对象外，重绘不丢）。回车保存并保持输入行打开（可连续添加），Esc 取消，失焦保存非空内容；失焦回调延后一拍并检查 `input.isConnected`，避免重绘换掉输入框时被误当成提交。
- **轻提示队列**：表格底部一条队列，FIFO 一次显示一条（info 3s / error 8s），几秒后自动消失，不阻塞任何操作。一次操作带回多条提示时不会互相顶掉。

## 边界情况

- 空项目 / 空分组：显示空状态文案。**用户创建的空模块/子模块仍然显示**（计数 `0/0`）——结构是用户建的，藏起来会让人以为没建成；只有「未分类」这类**派生的**分组在空时不渲染。
- 「待办」视图：没有模块树、没有分组行，表格直接平铺（与目标文档一致）。
- 「未分类」：项目内 `module_id IS NULL` 的任务聚成一个固定分组行，排在模块分组之后。
- 编辑任务文字为空白：还原原文，不写库。
- 迁移失败：拒绝启动并显示备份路径，绝不带着半迁移的库继续跑。

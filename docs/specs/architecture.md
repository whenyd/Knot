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
  project_id     INTEGER REFERENCES project(id) ON DELETE CASCADE,    -- NULL = 「我的待办」
  module_id      INTEGER REFERENCES module(id) ON DELETE CASCADE,
  submodule_id   INTEGER REFERENCES submodule(id) ON DELETE CASCADE,
  parent_task_id INTEGER REFERENCES task(id) ON DELETE CASCADE,       -- NULL = 顶层任务
  title          TEXT    NOT NULL,
  status         TEXT    NOT NULL DEFAULT 'todo' CHECK (status IN ('todo','doing','done')),
  priority       TEXT    NOT NULL DEFAULT 'mid'  CHECK (priority IN ('high','mid','low')),
  note           TEXT    NOT NULL DEFAULT '',
  created_at     TEXT,                                              -- 本地时间 'YYYY-MM-DDTHH:MM'，创建时写入
  start_at       TEXT,                                              -- 本地时间 'YYYY-MM-DDTHH:MM'，NULL = 未设置
  end_at         TEXT,                                              -- 同上；两项都有时 end_at >= start_at
  sort_order     INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_task_project ON task(project_id);
CREATE INDEX idx_task_module  ON task(module_id);
CREATE INDEX idx_task_parent  ON task(parent_task_id);
```

几个刻意的选择：

- **「我的待办」不是一行项目记录**，而是 `project_id IS NULL`。省掉一条"永远存在的系统记录"，也让"项目列表"查询天然干净。
- **子任务不存 `module_id`/`submodule_id`**（恒为 NULL）。子任务跟随父任务、不能单独移动，所以归属的唯一真相在父任务上；冗余一份只会漂移。
- **子任务优先级不高于父任务**：新建子任务默认"中"，父任务更低时取父任务的档位；把子任务调到高于父任务会被拒绝；父任务降档时，同一事务内把仍高于它的子任务一并降档。历史数据由 v3 迁移归一。
- **状态与优先级存文本而非整数**，用 `CHECK` 约束。库文件是给人看的，`'doing'` 比 `1` 好读，值集合也由 `CHECK` 兜住。
- **时间字段都存本地时间文本** `YYYY-MM-DDTHH:MM`，单人为本机工具，不需要跨时区换算；固定格式下字符串比较就是时间先后。**创建时间**由 `create_task` 写一次，之后不变；v5 之前的旧任务为 `NULL`。**开始 / 结束时间**由状态变化自动写入，没有手动设置命令。
- **"只允许一层子任务"无法用 `CHECK` 表达**（SQLite 的 CHECK 不支持子查询），由 `domain.rs` 在创建时保证。
- 外键级联一律是"删父带走子"：删项目带走它的模块与任务，删模块带走它的子模块与任务，删任务带走它的子任务。**删除模块时组内的任务是真的被删掉，不是"退回默认"**（两者都是数据，但只有一个是可恢复的，所以删除一律走 `plan_*` 先告知条数）。

> 注意：v1 迁移发布时这三列写的是 `ON DELETE SET NULL`（“退回默认”）。已发布的迁移条目不能改，所以 v2 用"新建表 → 拷贝 → 删旧表 → 改名"把 `module_id`/`submodule_id` 改成 `CASCADE`（SQLite 不支持 ALTER 外键）。

## 迁移策略

- **版本号**：`PRAGMA user_version`，`MIGRATIONS: &[&str]` 从第 1 条起顺序执行；`user_version` 即"已应用到第几条"。
- **时机**：打开连接后、任何读写之前，整段包在**单个事务**里。失败则回滚、拒绝启动，并在界面上给出备份文件路径。
- **只写前进**：不写 down 迁移。down 长期没人执行，是假的安全感；回滚靠下面的备份。
- **迁移前自动备份**：仅当 `0 < user_version < MIGRATIONS.len()`（已有数据的旧库升级）时，把库文件复制为同目录的 `tasks.sqlite.bak-v<旧版本>-<epoch 毫秒>`；新库和同版本启动不备份。备份失败即停止迁移，绝不无备份地改结构。每次备份后只保留最新 5 份，清理只认 `tasks.sqlite.bak-v*` 前缀（用户手工放的 `.bak-import-*` 不碰）。迁移失败时错误信息把**备份路径放在最前面**，再附一句原因。
- **外键开关**：每个连接显式 `PRAGMA foreign_keys = ON`（bundled SQLite 默认也开，但代码不依赖编译选项）。迁移事务内先关（重建表时外键会挡路），`PRAGMA foreign_key_check` 在**提交之前**执行——校验失败连 `user_version` 一起回滚，下次启动还会重新失败并拒绝启动，而不是误以为迁移已完成。
- **启动失败界面**：setup 失败不再 panic，而是隐藏主窗口、新建标题为「无法启动 — 我的待办」的错误窗口，加载 `index.html?startupError=<percent-encoded 原因>`。前端 `boot()` 在渲染任何界面、调用任何命令之前读取该参数，用独立页面原样显示原因与备份路径（`textContent`，不拼 HTML）。
- **开发期重置**：仅 dev build 的 invoke 注册里包含 `reset_database`（`#[cfg(debug_assertions)]`），前端也只在 `import.meta.env.DEV` 时显示左栏底部的「重置数据库（开发）」按钮。重置整段持锁：用内存连接换出文件连接并释放句柄 → 删 `tasks.sqlite` 与 `-journal`/`-wal`/`-shm` 边车 → 重新打开、跑迁移并放回。**保留 `.bak-*` 备份**；删除主文件失败时把原库接回去并返回错误，应用继续可用。正式版不注册这条命令。

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

## 时间自动记录

**创建时间**在 `create_task` 里与任务本体一起写一次，之后不变；升级到 v5 之前的任务没有这个值（`NULL`），迁移不伪造。

**开始 / 结束时间**不是独立可编辑的字段，而是**状态写入的副产品**：`domain.rs` 的 `write_status` 是唯一改状态的入口，也在同一个 `UPDATE` 里维护两项时间。

| 状态变化 | `start_at` | `end_at` |
|---|---|---|
| 未完成 → 进行中 | 为空则记当前分钟 | 清空 |
| 进行中 / 未完成 → 已完成 | 不动 | 记当前分钟 |
| 任何状态 → 未完成 | 清空 | 清空 |
| 同一状态重复写入 | 不动 | 不动 |

聚合自动改动父任务（自动完成 / 自动恢复进行中）也走 `write_status`，所以同样维护时间；一次级联完成的所有子任务共用同一个时间戳。任务如果在没开始过的情况下被级联或聚合标为已完成，只会得到结束时间，开始时间保持 `NULL`。

## 命令清单

所有写操作都返回受影响的行/节点，前端据此全量重取当前容器（数据量小，不做增量 patch）。“先确认再写”统一成 `plan_*` / `apply_*` 两段，前端不重算规则：

```
// 读
list_projects()                       -> [{ id, name, counts, is_todo }]   // 首项是虚拟的「我的待办」
list_modules(project_id)              -> [{ id, name, submodules: [...] }]
list_tasks(container)                 -> [task]   // container = { kind: 'todo' } | { kind: 'project', id }

// 写 · 任务
create_task({ container, module_id?, submodule_id?, parent_task_id?, title }) -> { task, notices }
rename_task(id, title)                              // 就地编辑；trim 后为空则拒绝
set_note(id, note)
set_priority(id, priority)                            // 子任务不能高于父任务；父任务降档级联下调子任务
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

**归属参数归后端校验**：`create_task` 接受 `module_id`/`submodule_id`，由 `domain.rs` 验证「模块属于该项目」「子模块属于该模块」「「我的待办」的任务不能带模块」。前端只负责把当前范围翻译成这两个 id（`resolveTarget`），不代替后端判定合法性。

**子任务的归属规则**：给了 `parent_task_id` 时，后端拒绝「父任务已有父任务」（只允许一层）与「父任务不在当前容器」；子任务落库时 `module_id`/`submodule_id` 一律为 NULL（调用方传了也忽略），`project_id` 跟随父任务，排序号按该父任务下 `MAX+1` 单独排队。父任务的分类就是子任务的分类，不需要也不允许冗余一份。

**默认值归属**：`status`/`priority` 的默认值由 `domain.rs` 在 INSERT 时显式写出，schema 的 `DEFAULT` 只作兜底。同一条规则不在两处表达，改默认值只有一个地方要动。

## 前端结构

- **视图状态**：`container`（我的待办 / 某个项目）、`scope`（全部 / 某模块 / 某子模块 / 默认）、`statusFilter: Set<Status>`、`priorityFilter: Set<Priority>`、`collapsedNodes: Set<nodeId>`（左栏范围树收起的模块）。筛选在本次会话内跨范围、跨项目保留。
- **一次渲染只建一棵树**：`buildGroups` 的结果同时给左栏范围树算计数、给表格做分组，`selectScope` 再把范围套上去——同一棵树不会被算两遍、更不会两份不一致。
- **两处折叠互不影响**：左栏范围树的收起（`collapsedNodes`）只管树的显示，表格分组行的收起（`collapsedGroups`）只管表格的行。一个是导航，一个是阅读密度。
- **数据管线**（单一方向，不双向绑定）：`list_tasks` → `buildGroups` → `selectScope` → 渲染时按两个筛选集判断任务是否出现；分组与计数始终使用未筛选的树，组内任务按 `priority` 再 `sort_order` 排序。
- **表格列固定**为 状态 / 任务 / 优先级 / 创建 / 开始 / 结束 / 备注。任务标题与备注摘要统一 13px；三列时间是**只读**摘要（`YYYY-MM-DD HH:MM`，13px 等宽数字，未记录为 `—`），没有点击编辑入口；子任务行同样使用这三列。时间不影响排序、筛选和计数。
- **列宽可调**：列与列之间有常驻细线（表头深一点、正文浅一点），表头每条分界线都是拖动把手；拖动时**同时调整线两侧的列**，总宽度不变。悬停时两侧表头一起高亮，拖动时浮出两列当前宽度（如 `创建 152px · 开始 80px`），避免以为只动了单列。「任务」列始终吃掉剩余宽度，窗口缩放时只有它伸缩。其他列宽存在 `localStorage`（`todolist.columnWidths.v1`），下次启动沿用；存储不可用时退回默认宽度。最小列宽只保证内容不消失，时间列被拖窄时以省略号截断，悬停仍有完整值。
- **纯函数单测**：`src/grouping.test.ts` 用 vitest 覆盖 `sortTasks` / `buildGroups` / `selectScope` / `resolveTarget` / `countGroup` 与筛选函数，`npm test` 直接跑，不启 Tauri、不碰 DOM。
- **筛选语义**：状态与优先级是“且”关系，集合内是“或”关系，允许某一组全不选。父任务不命中但子任务命中时，父任务以 `.is-context` 灰显保留为上下文，子任务正常显示；二者都不命中才隐藏整个父任务树。筛选空态不删除用户创建的分组，也不改变任何计数。
- **交互归属**：状态格点击只调 `plan_set_status`；`needs_confirm` 为真就弹窗；`apply_set_status` 回来后重取。就地编辑（双击）走 `rename_task`，空值前端还原、后端也拒绝。
- **弹窗共用同一个 modal**：`askConfirm`（告知影响后确认）、`askText`（改名/新建时输入一个名字）与 `askNote`（多行备注）。**同一时刻只能有一个弹窗**——新弹窗会把上一个按“取消”收场，否则两个 `await` 会同时悬着、抢同一批 DOM 监听，一次回车跑两个命令（已发生过）。输入框里空白名不关窗也不出库。
- **内联新建项目表单**是弹窗以外唯一的输入形态（左栏「＋ 新建项目」）。它只有一个收场出口 `resetProjectForm()`（隐藏 + 清空）：提交成功、点表单外（`document` 上的 `mousedown`，表单内不关）、Esc 都走它；打开时挂监听、关闭时摘掉，不留常驻监听。已输入的文字跟随表单一起丢弃——不保留草稿，也不计入状态。
- **菜单只有一份实现**：`src/menu.ts` 负责弹出、定位、关闭，三处共用——左栏项目/范围节点的右键菜单、任务行的右键菜单、优先级的 `.menu` 浮窗。入口只给 `items` 和位置（指针点或元素锚点），菜单项的行为由调用方提供。任务行右键给「移动到…」（仅顶层任务）与「删除任务」；子任务只有删除。带 `children` 的条目（目前只有「移动到…」）不内联展开，而是悬停/点击后在父行右侧弹出独立的 `.menu` 子面板；右侧放不下翻到左侧，父面板布局不重排。一次只保留一条面板路径：打开同级子面板会先关掉旧的；根面板滚动会关掉子面板。打开时先关掉上一个；点外面 / Esc / 滚屏 / 窗口失焦关闭；上下键在当前面板内循环，右键进子面板、左键退回父行，Esc 把焦点还给打开它的按钮。锚点定位在下方放不下时翻到上方，并夹在视口内。原生 `contextmenu` 在应用内默认被 `preventDefault()`（`boot()` 里统一挂一次），只有左栏项目/范围节点自己的 `contextmenu` 监听会打开上述菜单。
- **删除的回落**：删除任务只是重取当前容器；删除模块/子模块时，如果当前 `scope` 正落在被删的子树里，回到项目「全部」；删除项目时，如果当前 `container` 正是它，回到「我的待办」。删除前都用 `plan_*` 的条数填确认框，不在前端重算级联范围。
- **子任务渲染**：`buildGroups` 继续跳过 `parentTaskId !== null` 的任务（它们不占分组行，分组计数只算父任务）；渲染时另建 `parentTaskId → tasks` 的索引，父任务行后按同一套排序（优先级 → `sort_order`）渲染缩进子行。父任务行默认展开，`collapsedTasks` 只记本次会话里收起的那几个；父任务标题前的折叠箭头与行内「＋」只出现在顶层任务上。
- **行内子任务输入**：`addingSubtaskFor` 同时只有一个父任务，输入草稿放在 `subtaskDraft`（状态对象外，重绘不丢）。回车保存并保持输入行打开（可连续添加），Esc 取消，失焦保存非空内容；失焦回调延后一拍并检查 `input.isConnected`，避免重绘换掉输入框时被误当成提交。
- **轻提示队列**：表格底部一条队列，FIFO 一次显示一条（info 3s / error 8s），几秒后自动消失，不阻塞任何操作。一次操作带回多条提示时不会互相顶掉。
- **启动失败页**：`boot()` 的第一件事是读 `?startupError=`；有值就用 `document.body.replaceChildren` 换成独立错误页，原因与备份路径原样放进 `<pre>`，直接 return，连 `render()` / `reload()` 都不走。
- **开发版重置**：按钮在左栏底部，默认 `hidden`，只有 `import.meta.env.DEV` 为真时取消隐藏并绑定点击；`vite build` 会把整段分支裁掉，正式版的后端也没有这条命令。确认框走 `askConfirm`，成功后把视图状态复位到「我的待办 / 全部」并全量重取，无需重启进程。

## 边界情况

- 空项目 / 空分组：显示空状态文案。**用户创建的空模块/子模块仍然显示**（计数 `0/0`）——结构是用户建的，藏起来会让人以为没建成；只有「默认」这类**派生的**分组在空时不渲染。
- 「我的待办」视图：没有模块树、没有分组行，表格直接平铺（与目标文档一致）。
- 「默认」：项目内 `module_id IS NULL` 的任务聚成一个固定分组行，排在模块分组之前。
- 编辑任务文字为空白：还原原文，不写库。
- 任务从未进入进行中就被级联 / 聚合标为已完成：只记结束时间，开始时间保持 `NULL`（它确实没有开始过），不伪造开始时间。
- 升级前的旧任务没有创建时间：显示 `—`，不拿迁移时刻冒充创建时间。
- 迁移失败：事务回滚，库保持在旧版本；主窗口换成错误窗口，显示原因与迁移前备份的路径，绝不带着半迁移的库继续跑。修好原因后下次启动会重新尝试迁移。
- 启动失败页只渲染、不调用任何命令；错误窗口不托管数据库连接。

# 后续任务

后续所有工作的执行参考。依据：`goal-and-requirements.md`（要什么）、`architecture.md`（怎么分层、命令清单）、以及当前代码的真实状态。

新改一条能力前，先看这里对应任务前面有没有未定的「待决问题」；改完把规格补进 goal/architecture，再回来把任务勾掉。

---

## 现状

**已实现（22 条发布命令 + 1 条仅开发版命令）**

```
list_projects / create_project / rename_project / plan_delete_project / delete_project
list_modules / create_module / create_submodule / rename_module / rename_submodule
plan_delete_node / delete_node
list_tasks / create_task / rename_task / set_note / move_task / set_priority
plan_set_status / apply_set_status
plan_delete_task / delete_task
reset_database   // 仅 dev build 注册；release 不注册、前端不显示入口
```

界面：左栏（待办 + 项目 + 范围树 + **右键菜单**）、固定按模块分组的表格（分组折叠、子树计数）、
状态点击循环三态、优先级浮窗、双击就地改名、**新建任务落入当前范围**、应用内确认框、输入弹窗与
**多行备注弹窗**、**任务行尾「…」菜单**（移动到任意归类 + 删除）、
**三级结构删除**（确认框告知连带任务数，删除后自动回落视图）、
**一层子任务**（父行「＋」行内连续添加、缩进 4 个空格、默认展开可折叠）、
**父子状态聚合**（三处事件触发、级联确认、轻提示 FIFO 排队）、
**状态 / 优先级筛选**（默认隐藏已完成、父子任务独立命中、未命中父任务灰显为上下文、全量计数）、
**数据安全**（迁移前自动备份保留 5 份、迁移失败错误窗口给原因与备份路径、开发版删库重建）。

**规格里有、代码里没有**

| 能力（goal-and-requirements） | 缺口 |
| --- | --- |
| 三级结构可新建 | ✅ 已完成（阶段一 T1/T2） |
| 三级结构可改名 | ✅ 已完成（阶段一 T3） |
| 删除（任务 / 模块 / 子模块 / 项目） | ✅ 已完成（阶段二 T4–T7） |
| 归类与跨容器移动 | ✅ 已完成（阶段三 T9） |
| 备注 | ✅ 已完成（阶段三 T8） |
| 子任务（一层） | ✅ 已完成（阶段四 T10/T11） |
| 父子状态联动 | ✅ 已完成（阶段四 T12–T14） |
| 状态 / 优先级筛选 | ✅ 已完成（阶段五 T15/T16） |
| 迁移安全 | ✅ 已完成（阶段六 T17–T19） |
| 工程化 | ✅ 前端单测（T20）；release exe 实跑（T21，安装包按归档约定跳过） |

---

## 待决问题

### D1 · 删除模块时，里面的任务去哪？—— **已定：确认后一并删除**

- 结论：删除模块/子模块时，组内任务**真删**（先弹窗告知条数），不退回未分类。
- 已同步：`goal-and-requirements.md`「删除」去掉了“（推断）”；`architecture.md` 的 schema 把 `task.module_id` / `submodule_id` 改成 `ON DELETE CASCADE`，并写明为什么需要一条 v2 迁移（v1 已发布为 `SET NULL`，历史条目不能改）。
- 连带结论：删项目同样是“带走它的所有任务”——`project_id` 本来就是 `CASCADE`，无需迁移。

### D2 · 子模块与模块的新建入口 —— **已定：左栏右键菜单**

- 项目行右键 → 新建模块；模块行右键 → 新建子模块。
- 可做：`contextmenu` 在 Tauri 的 webview 里就是普通 DOM 事件，`preventDefault` 掉原生菜单、弹自建菜单即可（与确认框同一套 DOM 层）。

### D3 · 项目名 / 模块名怎么改 —— **已定：同一处右键菜单里的「重命名」**

- 不占用双击（双击归任务标题），也不占用单击（单击已经被“切换视图”用了）。
- 重命名走弹窗输入，不复用任务的行内编辑：结构名不常改，不值得为它再写一套行内编辑。

### D4 · 筛选落地时默认视图会变

goal 规定状态标签**默认只勾「未完成」「进行中」**，也就是说筛选做完之后，“已完成”默认**隐藏**，而现在它们一直显示（灰显 + 删除线）。这是规格要求的行为，到时候别当成 bug。

### D5 · 右键不可发现（未确认，建议的补救）

右键菜单没有任何视觉提示，第一次用的人不会知道能右键；纯键盘用户也不友好。建议：同一菜单同时挂在**鼠标悬停时出现的 `⋯` 按钮**上（一个菜单组件、两个触发器），成本很小。**尚未确认。**

---

## 任务

编号即建议执行顺序；同一阶段内可并行，「依赖」列指明不能提前的。

### 阶段 1 · 左栏菜单与结构维护

| # | 任务 | 改动位置 | 验收 | 依赖 |
| --- | --- | --- | --- | --- |
| ~~T1~~ | ✅ **左栏右键菜单骨架**（已完成，`src/menu.ts`） | `main.ts`：自建菜单组件（`contextmenu` → `preventDefault` → 定位 → 关闭：点外面 / Esc / 滚屏）；项目行与范围节点各自挂上 | 右键项目行、模块行、子模块行都能弹出；同一时刻只有一个菜单；菜单不会跑出视口 | — |
| ~~T2~~ | ✅ **新建模块 / 子模块**（已完成；工具栏那个「＋ 模块」按钮已移除，入口统一到右键） | `domain::create_submodule`（校验模块存在、排序号同组 `MAX+1`）+ `commands` + `api.ts` + T1 的菜单项 | 建完左栏范围树与表格子分组行都出现；重名不拦（规格未要求唯一） | T1 |
| ~~T3~~ | ✅ **重命名项目 / 模块 / 子模块**（已完成，走弹窗输入） | `domain::rename_project` / `rename_module` / `rename_submodule`（复用 `require_name` 的 trim + 拒空）+ 菜单项 → 弹窗输入 | 空白名被拒且库不变；左栏与表格分组行同步更新 | T1 |

### 阶段 2 · 删除（两段式）— 已完成

一律 `plan_*` / `apply_*` 形状，确认框复用现有的 `askConfirm`，入口复用 T1 的菜单。

| # | 任务 | 改动位置 | 验收 | 依赖 |
| --- | --- | --- | --- | --- |
| ~~T4~~ | ✅ **删除任务**（已完成） | `plan_delete_task(id) -> { taskCount }` / `delete_task(id)` + 任务行尾「…」菜单（第一版只有删除） | 确认框告知条数（含子任务）；删完列表少一行 | — |
| ~~T5~~ | ✅ **删除模块 / 子模块**（已完成） | **迁移 v2**：新建 task 表 → 拷数据 → 删旧表 → 改名 → 重建索引，把 `module_id` / `submodule_id` 改成 `ON DELETE CASCADE`；然后 `plan_delete_node(kind, id)` / `delete_node(kind, id)` + 菜单项（`kind = module` / `submodule`） | 存量库能升到 v2 且 `foreign_key_check` 通过；删模块时它的子模块与任务一并消失；条数包含子模块里的任务；若正停在被删范围则回落到「全部」 | D1、T1 |
| ~~T6~~ | ✅ **删除项目**（已完成） | `plan_delete_project(id)` / `delete_project(id)` + 菜单项 | 条数含全项目任务；删完若正停在该项目则回落到「待办」 | D1、T1 |
| ~~T7~~ | ✅ **把剩下两处菜单并进 `src/menu.ts`**（已完成） | `src/menu.ts` 已是通用组件（T1 建好）；任务行菜单（T4 引入）直接用它；优先级浮窗（`main.ts` 里原来的 `.picker`）也改用它 | 只有一份“弹出/定位/关闭”的实现；优先级浮窗的方向键与 Esc 行为不变 | T1、T4 |

### 阶段 3 · 备注与归类移动 — 已完成

| # | 任务 | 改动位置 | 验收 | 依赖 |
| --- | --- | --- | --- | --- |
| ~~T8~~ | ✅ **备注弹窗**（已完成） | `domain::set_note`（原样落库，空串=清空）+ 点备注格弹窗（复用 modal 骨架，`askNote` 用文本域，Enter 是换行） | 任意文本（含换行）保存后表格显示单行截断摘要；空备注显示 `—` | — |
| ~~T9~~ | ✅ **归类移动**（已完成） | `domain::move_task(id, target)`（归属校验与 `create_task` 共用 `require_task_target`；子任务跟随父任务换容器）+「…」菜单 →「移动到…」二级菜单 | 双向可移动；移动后 `sort_order` 取目标组末尾；跨容器时两侧视图都要重取 | T7 |

### 阶段 4 · 子任务与状态联动 — 已完成

这一阶段内部**顺序不能乱**：T10 / T11 没有，T12 就没有数据可联动。

| # | 任务 | 改动位置 | 验收 | 依赖 |
| --- | --- | --- | --- | --- |
| ~~T10~~ | ✅ **新建子任务**（已完成） | `create_task` 增加 `parent_task_id`，返回 `{ task, notices }`；后端校验一层限制与容器一致，子任务强制 `module_id`/`submodule_id = NULL`，排序号按父任务分组 `MAX+1` | 子任务不能挂子任务（后端拒）；子任务不出现在任何分组行里 | — |
| ~~T11~~ | ✅ **子任务渲染**（已完成） | `main.ts` 按 `parentTaskId` 建索引，在待办平铺与分组表格里都渲染缩进子行；父行折叠箭头默认展开，行内「＋」加子任务（回车连续添加、Esc 取消、失焦保存非空） | 分组计数仍只算父任务 | T10 |
| ~~T12~~ | ✅ **父子状态聚合**（已完成） | `domain.rs` 内 `aggregate_parent_status` + 三处触发（子状态变、子新增、子删除），同一事务；字面规则含零子任务空真；`showNotice` 改为 FIFO 队列 | 轻提示出现且不阻塞；**手动退回父任务时子任务不动**；多条提示排队而不是互相顶掉 | T10、T11 |
| ~~T13~~ | ✅ **级联确认接活**（已完成） | 前端既有 `needs_confirm` 分支与 `askConfirm` 在 T10 之后第一次真的能被触发；补测试 | 父任务有未完成子任务时标完成 → 弹窗告知条数 → 确认后一并完成 | T10 |
| ~~T14~~ | ✅ **删除子任务走聚合**（已完成） | `delete_task` 改为事务内删除 + 聚合，返回 `DeleteOutcome { notices }` | 删掉最后一个未完成子任务 → 父变 `done` + 轻提示 | T12、T4 |

阶段 4 实现决策：

- 子任务入口用父任务行的 `＋` 行内输入，回车后输入行保持打开，可连续添加；父任务行有折叠箭头、默认展开，收起状态只在本次会话内记住。
- 聚合按字面规则实现：零个子任务时 `all()` 为真，所以删除最后一个未完成子任务会让父任务变 `done`。
- 子任务优先级以父任务为上限：新建时默认"中"、父任务更低则跟随；父任务降档时级联下调仍高于它的子任务，旧数据由 v3 迁移归一。
- `create_task` / `delete_task` 的返回结构改为带 `notices` 的对象；只有父任务被自动改动时才产生提示。
- 轻提示改为 FIFO 队列，一次显示一条（info 3s / error 8s）。

### 阶段 5 · 筛选 — 已完成

| # | 任务 | 改动位置 | 验收 | 依赖 |
| --- | --- | --- | --- | --- |
| ~~T15~~ | ✅ **顶部两组标签**（已完成） | `main.ts` 工具栏 + `state.statusFilter` / `state.priorityFilter` + `grouping.ts` 过滤函数（与现有 `selectScope` 串在同一条管线上） | 状态默认只勾未完成/进行中，优先级全勾；两组与范围**叠加**生效 | — |
| ~~T16~~ | ✅ **已完成的表现收尾**（已完成） | 同上；父任务不命中但子任务命中时，父行以灰显上下文保留，子任务独立筛选 | 勾着「已完成」时灰显 + 删除线；取消勾选时行消失但**分组与范围树计数仍显示**（如 `3/7`） | T15 |

阶段 5 实现决策：

- 默认视图从“看得见已完成”变为“看不见已完成”（D4，已落地）。
- 状态与优先级筛选在本次会话内跨范围、跨项目保留，不写入数据库。
- 父任务不命中但任一子任务命中时，父任务灰显为上下文，只渲染命中的子任务；父子都不命中才整棵隐藏。
- 筛选允许一组全不选并显示筛选空态；用户创建的分组仍保留，计数始终基于未筛选数据。

### 阶段 6 · 数据安全 — 已完成

| # | 任务 | 改动位置 | 验收 | 依赖 |
| --- | --- | --- | --- | --- |
| ~~T17~~ | ✅ **迁移前自动备份**（已完成） | `db.rs` / `migrations.rs`：旧库升级前复制成 `tasks.sqlite.bak-v<旧版本>-<epoch 毫秒>`，只保留最近 5 份 | 旧 `user_version` 的库启动 → 生成备份；连续 6 次只剩 5 份；手工 `.bak-import-*` 不被清理 | — |
| ~~T18~~ | ✅ **迁移失败不裸崩**（已完成） | `lib.rs` setup 不再 `?`/panic：隐藏主窗口、开错误窗口，加载 `index.html?startupError=…`；前端 `boot()` 首行接管渲染 | 失败时窗口直接显示备份路径与原因，不调用任何命令；库保持旧版本 | T17 |
| ~~T19~~ | ✅ **dev 删库重建**（已完成） | 仅 debug 注册 `reset_database` + 左栏底部按钮（仅 `import.meta.env.DEV` 显示）；整段持锁换连接、删活库、重建 | 正式版不注册命令也不显示按钮；开发版点完视图回到空库 | — |

阶段 6 实现决策：

- 备份只在 `0 < user_version < 最新版本` 时生成；新库与同版本启动不备份。备份失败即停止迁移。
- `PRAGMA foreign_key_check` 移进迁移事务、提交之前；失败连 `user_version` 一起回滚，下次启动仍会拒绝。
- 错误窗口用 `WebviewUrl::App("index.html?startupError=…")` 独立创建，不依赖 setup 阶段 `window.url()` 的时序；原因只取 SQLite 报错的第一行，备份路径放在最前面，两者都无需滚动即可看见。
- 删库重建不重启进程：持锁换出连接 → 删主文件与边车 → 重新打开并迁移 → 放回；保留 `.bak-*` 备份，主文件删不掉时原库接回。
- 新增单测覆盖备份生成/保留 5 份/失败带路径/重置重建/FK 校验回滚；全部 88 个后端测试通过。

### 阶段 7 · 工程化

| # | 任务 | 改动位置 | 验收 | 依赖 |
| --- | --- | --- | --- | --- |
| ~~T20~~ | ✅ **前端纯函数单测**（已完成） | 引入 vitest 5；`src/grouping.test.ts` 覆盖 `grouping.ts` 的 `sortTasks` / `buildGroups` / `selectScope` / `resolveTarget` / `countGroup`（另含筛选函数） | `npm test` 绿（14 条）；`resolveTarget` 的子模块用例会守住“必须同时给出 moduleId”，故意改坏实现时该用例变红 | — |
| ~~T21~~ | ✅ **release 构建实跑**（已完成；安装包按归档约定跳过） | `npx tauri build --no-bundle`（无 bundler）；release exe | 启动 release exe 一次：CDP 创建项目/模块/任务后删除模块，`plan.taskCount=1`，库中 `tasks_in_module=0`、`total_tasks=0`、`foreign_key_check=[]`、`user_version=3`；库路径 `%APPDATA%\todolist\tasks.sqlite`。系统无 WiX/NSIS，项目已归档，不再下载 bundler | T17 |
| T22 | （可选）CI | GitHub Actions：`cargo test` + `tsc` + `vitest` | 每个 PR 自动跑 | T20 |

阶段 7 实现决策：

- 前端单测用 vitest 5：`npm test` 就是 `vitest run`，配置在 `vitest.config.ts`，只收 `src/**/*.test.ts`，跑在 node 环境，不启 Tauri、不碰 DOM。
- 测试数据用工厂函数直接造 `Task` / `Module`；`resolveTarget` 的子模块用例同时断言 `moduleId` 与 `submoduleId`，作为“实现退化成只给 submoduleId 就会红”的守门用例。
- 测试文件放在 `src/` 下，因此 `tsc --noEmit` 也会检查它；`vite build` 不会把测试打进产物。
- T21 只用系统已有能力：本机没有 WiX/NSIS，项目已归档，不再下载 bundler；改用 `npx tauri build --no-bundle` 验证 release exe 与库/FK 行为，安装包部分跳过。

---

## 验证约定

每个任务收尾都按这套走，别只报“编译通过”：

1. `cargo test`（后端规则）+ `npm test`（前端纯函数）+ `npx tsc --noEmit`（前端类型）+ `vite build`。
2. **涉及界面的改动**：用 `@tauri-apps/api/mocks` 的 `mockIPC` 把 `src/main.ts` 挂到 `index.html` 的结构上，起 `npx vite --port 1420`，再用无头 Edge 截图人工核对。
   - 需要验证调用链时，让 mock 后端**有状态**（`create_task` 真的往数组里追加、`list_tasks` 真的返回它），脚本模拟点击/输入。
   - **收尾时按端口找 PID 再 `taskkill //F //PID`**：`background_command stop` 杀不掉 `npx` 拉起的子进程，会占着 1420 让 `tauri dev` 起不来。
3. 真实窗口 `npm run tauri dev` 过一遍交互——截图能证明渲染，证明不了“点下去之后数据对不对”。
4. 收尾把规格改动写回 `goal-and-requirements.md` / `architecture.md`，再更新本文件的状态。

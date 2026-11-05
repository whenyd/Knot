# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 项目

单人、本机、不联网的桌面待办（Tauri 2 + 原生 DOM/TypeScript + SQLite）。规格分三份，动手前先读：

- `docs/specs/goal-and-requirements.md` — 要什么（能力、取舍、Non-Goals）
- `docs/specs/architecture.md` — 怎么分层、数据模型、命令清单、交互约定
- `docs/ROADMAP.md` — 还剩什么、依赖顺序、**动手前必须确认的待决问题（D1–D5）**

规格驱动：改行为前先看 ROADMAP 对应任务有没有未决问题；改完把规格写回 goal/architecture，再勾掉 ROADMAP。README 的「当前状态」一节已滞后（仍写 14 条命令、不能删除），以 ROADMAP 为准。

## 常用命令

```bash
npm install
npm run tauri dev                          # 开发窗口；Vite 固定占用 1420（strictPort）
npm run build                              # tsc + vite build（前端类型检查 + 打包）
npm test                                   # 前端纯函数单测（vitest）
npx tsc --noEmit                           # 只做类型检查
cargo test --manifest-path src-tauri/Cargo.toml             # 全部后端单测
cargo test --manifest-path src-tauri/Cargo.toml <测试名>     # 单个测试，如 v2_upgrades_existing_data_and_cascades
npm run tauri build                        # release 安装包（NSIS / MSI；需系统已有 WiX/NSIS）
```

前端测试用 vitest，测试文件是 `src/grouping.test.ts`，覆盖 `grouping.ts` 的排序、分组、范围、计数与筛选纯函数。后端测试仍内嵌在 `domain.rs` / `migrations.rs` / `db.rs` / `lib.rs` 的 `#[cfg(test)]` 里，用内存库或临时目录。

数据库：Windows `%APPDATA%/knot/tasks.sqlite`。删掉该文件即重置；dev 版左栏底部另有「重置数据库（开发）」入口（只清活库与边车，保留 `.bak-*` 备份），正式版不注册这条命令。

## 分层与硬性规矩

```
前端 (src/main.ts) → src/api.ts → commands.rs → domain.rs → db.rs / migrations.rs → SQLite
```

- **规则只住在 `domain.rs`**：状态联动、级联、排序号分配、归属校验都在这里，并用单测固定。`commands.rs` 是薄壳（解包参数 + 转发），不写 `if` 业务判断、不拼 SQL。SQL 触发器不重复表达规则。
- **`invoke` 只出现在 `src/api.ts`**。前端不直接碰 Tauri 命令，也不重算聚合规则——任何写操作之后一律 `reload()` 全量重取当前容器（数据量小，不做增量 patch）。
- **IPC 字段契约归 Rust**：结构体 `#[serde(rename_all = "camelCase")]`、枚举值小写，`api.ts` 的类型逐字对应。字段名只在一处定义；`domain.rs` 有专门盯这个契约的测试。
- **先告知、再写**：所有破坏性/联动操作走 `plan_*`（只读，返回 `needs_confirm` / 条数）→ 前端弹窗 → `apply_*` / `delete_*`。确认框里的数量来自后端，前端不重算级联范围。
- **错误是给人看的一句话**（`Result<T, String>`），前端原样显示在底部轻提示；需要分类处理的情况走 `Ok` 里的字段，不走错误通道。

## 数据模型与迁移

- 「我的待办」不是一条项目记录，而是 `task.project_id IS NULL`。
- 分类是 `project → module → submodule`；子任务只有一层（`parent_task_id`），且子任务不存 `module_id`/`submodule_id`——归属真相在父任务，后端拒绝“子任务挂子任务”。
- 任务带 `created_at` / `start_at` / `end_at`（本地时间文本 `YYYY-MM-DDTHH:MM`，未记录即 `NULL`），**都不手动编辑**：`create_task` 写一次创建时间；`domain.rs` 的 `write_status` 在改状态时维护开始 / 结束——进进行中记开始、进已完成记结束、退回未完成清空。时间只展示，不参与排序、筛选与计数。
- 删除一律“删父带走子”：项目/模块/子模块/父任务的外键都是 `CASCADE`。删除模块时组内任务是真删，不是退回默认（D1 已定）。
- 迁移用 `PRAGMA user_version`，`MIGRATIONS: &[&str]` 顺序执行。**已发布的条目一个字都不要改**（v1 把分类外键写成 `SET NULL` 的历史无法修改，v2 靠“建新表→拷贝→删旧表→改名”改成 `CASCADE`）。只写前进，不写 down。
- 迁移整段包在单个事务里；事务前关 `foreign_keys`（事务内是 no-op），`PRAGMA foreign_key_check` 在**提交之前**跑，失败连 `user_version` 一起回滚。每个连接打开时显式 `PRAGMA foreign_keys = ON`。
- 迁移前自动备份：仅旧库升级（`0 < user_version < 最新版本`）时生成 `tasks.sqlite.bak-v<旧版本>-<epoch 毫秒>`，保留最近 5 份（只清理 `bak-v*`）；备份失败即停止迁移。迁移失败由 `lib.rs` 开一个 `?startupError=` 错误窗口显示原因与备份路径，不再 panic。`reset_database` 只在 debug 注册、按钮只在 `import.meta.env.DEV` 显示，正式版没有这条路径。

## 前端结构

- `main.ts` 持有唯一一份可变 `state`（`container` / `scope` / `collapsedNodes` 等），所有渲染都是“全量状态 → 重建 DOM”，没有框架、没有双向绑定。
- `grouping.ts` 是纯函数数据管线：`sortTasks` → `buildGroups` → `selectScope` → `countGroup`；同一棵树同时供左栏范围树计数和表格分组，不建第二份。**分组计数只算父任务**（子任务跟随父任务渲染，不单独成组）。阶段 5 的筛选只作用于渲染，计数仍基于未筛选数据；父任务不命中但子任务命中时，父行灰显为上下文。
- `menu.ts` 是唯一的菜单实现（弹出/定位/关闭/键盘导航/子面板），三处共用：左栏右键菜单、任务行右键菜单、优先级浮窗。任务行的移动与删除只在右键菜单里；新增菜单不要另写一套。
- 列宽在 `main.ts` 里维护：表格不画单元格框线（列间无常驻竖线，行间无横线），只有表头下方一条分隔线；拖动表头分界线**同时调整两侧列**，悬停两侧表头高亮、拖动时浮出两列宽度、把手位置才显竖线；「任务」列吃掉剩余宽度；非任务列宽存 `localStorage`。
- 弹窗（`askConfirm` / `askText` / `askNote`）共用同一套 modal，**同一时刻只允许一个**——新弹窗会把上一个按取消收场，否则两个 `await` 抢同一批监听。
- 新建项目是唯一的非弹窗输入形态，只有一个收尾出口 `resetProjectForm()`；打开时挂 `document` 监听，关闭时摘掉。
- 删除后的回落：删模块/子模块时若 `scope` 在被删子树内 → 回项目「全部」；删项目时若正停在该项目 → 回「我的待办」。

## 界面验证约定

ROADMAP 末尾的验证约定是硬要求，不能只报“编译通过”：

1. `cargo test` + `npx tsc --noEmit` + `npm run build`。
2. 涉及界面的改动：用 `@tauri-apps/api/mocks` 的 `mockIPC` 把 `main.ts` 挂到 `index.html`，起 `npx vite --port 1420`，无头 Edge 截图人工核对；验证调用链时让 mock 后端有状态并模拟点击。
3. 真实窗口 `npm run tauri dev` 过一遍交互（截图证明渲染，证明不了数据对不对）。
4. 收尾按端口找 PID 再 `taskkill //F //PID`：`background_command stop` 杀不掉 `npx` 拉起的子进程，会占着 1420。

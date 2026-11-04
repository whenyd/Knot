# todolist

开发任务的结构化清单：单人、本机、不联网的桌面待办。
需求见 [goal-and-requirements.md](goal-and-requirements.md)，分层与数据模型见 [architecture.md](architecture.md)，
**剩下要做什么见 [ROADMAP.md](ROADMAP.md)**。

## 当前状态

骨架、编辑与结构维护已就位：建项目/模块/子模块 → 建任务（落进当前范围）→ 分组表格里改状态/优先级/文字 → 重启后仍在。

已实现 14 条命令：`list_projects` / `create_project` / `list_modules` / `create_module` / `create_submodule` /
`rename_project` / `rename_module` / `rename_submodule` / `list_tasks` / `create_task` / `rename_task` /
`set_priority` / `plan_set_status` / `apply_set_status`。

界面：左栏（待办 + 项目 + **范围树** + **右键菜单**：新建模块/子模块、重命名）、固定按模块分组的表格（分组折叠、子树计数）、
状态格点击循环三态、优先级格点开三档浮窗、双击标题就地改名（回车/失焦存、Esc 取消）、**新建任务落入当前范围**、
应用内确认框与输入弹窗。

结构能建能改名，但还不能删；任务能改状态/优先级/文字，但不能归类移动或写备注。

尚未实现的部分、它们的依赖顺序以及动手前必须先定的问题，全部列在 [ROADMAP.md](ROADMAP.md)，
不在本文重复维护。

## 结构

```
index.html            窗口骨架：左栏（待办 + 项目 + 范围树）+ 主区（工具栏 + 分组表格 + 添加行）
src/
  api.ts              唯一的前端 ↔ 后端接口层（invoke 只出现在这里）
  grouping.ts         纯函数数据管线：组内排序、按模块建树分组、范围筛选、分组计数
  grouping.test.ts    纯函数单测（vitest）
  menu.ts             左栏右键菜单组件（弹出/定位/关闭）
  main.ts             状态 + 事件 + 渲染（原生 DOM，无 UI 框架）
src-tauri/src/
  main.rs             入口，只调 todolist_lib::run()
  lib.rs              装配：建连接、托管状态、注册命令
  commands.rs         命令层：参数解包 + 转发，不含规则
  domain.rs           领域层：唯一写入口，所有规则住在这里（含单测）
  db.rs               存储层：库文件定位、连接、连接级 PRAGMA
  migrations.rs       schema 版本列表 + 迁移执行（含单测）
```

## 开发

```bash
npm install
npm run tauri dev      # 开发窗口（前端热更新）
npm test               # 前端纯函数单测（vitest）
npm run build          # 前端类型检查 + 打包
npm run tauri build    # 出安装包（需系统已有 WiX/NSIS；否则用 npx tauri build --no-bundle）
```

数据库文件：Windows `%APPDATA%/todolist/tasks.sqlite`（macOS/Linux 为对应平台的 app data 目录）。
删掉该文件即可重置。

## 分层规矩

- 规则只在 `domain.rs` 里表达，不用 SQL 触发器重复一遍。
- 前端不重算聚合规则，写操作之后一律全量重取当前容器。
- `invoke` 只出现在 `src/api.ts`。

# todolist

开发任务的结构化清单：单人、本机、不联网的桌面待办。
需求见 [goal-and-requirements.md](goal-and-requirements.md)，分层与数据模型见 [architecture.md](architecture.md)。

## 当前状态

框架已搭好，打通了一条最小纵向链路：**建项目 → 建任务 → 按模块分组的表格渲染 → 重启后仍在**。

已实现：`list_projects` / `create_project` / `list_modules` / `create_module` / `list_tasks` / `create_task`，
以及完整 schema 与 `user_version` 迁移框架。

尚未实现（见 architecture.md 的命令清单）：改名、移动归类、删除、子任务、
三态状态循环与父子联动、优先级编辑、备注弹窗、筛选、折叠、就地编辑、两段式 `plan_*` / `apply_*`、
迁移前自动备份与失败时的备份路径提示。

## 结构

```
index.html            窗口骨架：左栏（待办 + 项目）+ 主区（工具栏 + 分组表格 + 添加行）
src/
  api.ts              唯一的前端 ↔ 后端接口层（invoke 只出现在这里）
  grouping.ts         纯函数数据管线：组内排序、按模块建树分组、分组计数
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
npm test               # 领域层 + 迁移单测（在 src-tauri 下 cargo test）
npm run build          # 前端类型检查 + 打包
npm run tauri build    # 出安装包
```

数据库文件：Windows `%APPDATA%/todolist/tasks.sqlite`（macOS/Linux 为对应平台的 app data 目录）。
删掉该文件即可重置。

## 分层规矩

- 规则只在 `domain.rs` 里表达，不用 SQL 触发器重复一遍。
- 前端不重算聚合规则，写操作之后一律全量重取当前容器。
- `invoke` 只出现在 `src/api.ts`。

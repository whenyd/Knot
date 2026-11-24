---
id: i18n-glossary
type: reference
title: 中英术语表
status: active
---

# 中英术语表

中文版文档是权威版本。本表锁定界面与文档里的核心译名；改译名时同时改 `src/i18n/messages.en.ts`、
`src/i18n/backend.ts` 与英文文档。

## 界面术语

| 中文 | English | 说明 |
| --- | --- | --- |
| 我的待办 | My Tasks | 虚拟容器，不是真实项目；不用 "My To-dos" |
| 未开始 | Todo | 状态 token 恒为 `todo` |
| 进行中 | Doing | token 恒为 `doing` |
| 已完成 | Done | token 恒为 `done` |
| 高 / 中 / 低 | High / Medium / Low | token 恒为 `high` / `mid` / `low` |
| 默认 | Default | 未归类任务的派生分组 |
| 全部 | All | 项目范围 |
| 项目 | Project | |
| 模块 | Module | |
| 子模块 | Submodule | |
| 任务 | Task | |
| 子任务 | Subtask | |
| 父任务 | Parent task | |
| 状态 | Status | |
| 优先级 | Priority | |
| 创建 / 开始 / 结束 | Created / Start / End | 表格列名；tooltip 用 Created time / Start time / End time |
| 备注 | Note | |
| 筛选 | Filter | |
| 排序 | Sort | |
| 升序 / 降序 | Ascending / Descending | `aria-sort` token 不翻译 |
| 新建 | New | |
| 重命名 | Rename | |
| 删除 | Delete | |
| 移动到 | Move to | |
| 降为子任务 | Demote to subtask | |
| 升为父任务 | Promote to top level | |
| 主题 | Theme | |
| 浅色 / 深色 | Light / Dark | |
| 跟随系统 | System | 主题与语言模式共用 |
| 语言 | Language | |
| 关于 | About | |
| 无法启动 | Unable to start | |
| 数据库 | Database | |
| 迁移 | Migration | |
| 备份 | Backup | |
| 版本 / 平台 / 运行时 | Version / Platform / Runtime | |
| 许可证 / 版权 | License / Copyright | |

## 后端消息参数 token

这些值由 Rust 传、前端翻译，**不要**把中文写进参数：

| 参数 | 取值 | 中文 | English |
| --- | --- | --- | --- |
| `entity` | `project` / `module` / `submodule` / `task` | 项目 / 模块 / 子模块 / 任务 | Project / Module / Submodule / Task |
| `field` | 同上 | 项目名 / 模块名 / 子模块名 / 任务文字 | Project name / Module name / Submodule name / Task text |
| `noun` | `module` / `submodule` | 模块 / 子模块 | module / submodule |

## 不翻译的内容

- 状态、优先级、`Container` / `NodeKind` 等持久化 token。
- `data-*` 属性值、`aria-sort` 的 `ascending` / `descending` / `none`。
- `localStorage` 的 key 与值（`knot.*.v1`、`light/dark/system`、`zh/en/system`）。
- 用户输入的项目名、模块名、子模块名、任务标题与备注。
- 品牌名 `Knot`、时间存储格式 `YYYY-MM-DDTHH:MM`、SQLite/OS 原始错误详情。

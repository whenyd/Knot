// Chinese UI dictionary. This is the source of truth for message keys.
export const zh = {
  "app.title": "Knot",

  "common.cancel": "取消",
  "common.confirm": "确定",
  "common.save": "保存",
  "common.create": "创建",
  "common.ok": "确定",
  "common.delete": "删除",
  "common.dash": "—",
  "common.expand": "展开",
  "common.collapse": "收起",

  "language.label": "语言",
  "language.switch": "切换语言",
  "language.name.zh": "中文",
  "language.name.en": "English",
  "language.mode.system": "跟随系统",

  "theme.label": "主题",
  "theme.switch": "切换主题",
  "theme.light": "浅色",
  "theme.dark": "深色",
  "theme.system": "跟随系统",

  "nav.projects": "项目",
  "nav.projectMenu": "项目菜单",
  "nav.todo": "我的待办",
  "nav.todoCannotDelete": "「我的待办」不是项目，不能删除",
  "nav.newProject": "新建项目…",
  "nav.deleteProject": "删除项目",
  "nav.renameProject": "重命名项目",
  "nav.newModule": "新建模块",
  "nav.newSubmodule": "新建子模块",
  "nav.renameModule": "重命名模块",
  "nav.renameSubmodule": "重命名子模块",
  "nav.deleteModule": "删除模块",
  "nav.deleteSubmodule": "删除子模块",
  "nav.allProjects": "所有项目",

  "scope.all": "全部",
  "scope.default": "默认",
  "scope.module": "模块",
  "scope.submodule": "子模块",

  "status.todo": "未开始",
  "status.doing": "进行中",
  "status.done": "已完成",
  "status.tooltip": "{current}，点一下改为{next}",
  "status.aria": "状态：{status}",

  "priority.high": "高",
  "priority.mid": "中",
  "priority.low": "低",
  "priority.label": "优先级",
  "priority.tooltip": "优先级{level}（点击选择）",
  "priority.tooltipChild": "优先级{level}（点击选择；不能高于父任务）",

  "column.status": "状态",
  "column.title": "任务",
  "column.priority": "优先级",
  "column.created": "创建",
  "column.start": "开始",
  "column.end": "结束",
  "column.note": "备注",
  "column.resizeTip": "{left} {leftWidth}px · {right} {rightWidth}px",
  "column.resizeHandle": "拖动调整列宽（到最小后带动相邻列）",

  "sort.priority": "优先级",
  "sort.created": "创建时间",
  "sort.start": "开始时间",
  "sort.end": "结束时间",
  "sort.ascHint": "{label}升序，点击切换为降序",
  "sort.descHint": "{label}降序，点击恢复默认排序",
  "sort.defaultHint": "按{label}升序排序",

  "filter.status": "状态",
  "filter.priority": "优先级",
  "filter.groupAria": "{label}筛选",
  "filter.tasksAria": "任务筛选",

  "time.created": "创建",
  "time.start": "开始",
  "time.end": "结束",
  "time.createdLabel": "创建时间",
  "time.startLabel": "开始时间",
  "time.endLabel": "结束时间",
  "time.createdHint": "创建任务时自动记录",
  "time.stateHint": "随状态自动记录",
  "time.tooltip": "{label}：{value}（{hint}）",
  "time.notRecorded": "未记录",

  "task.addPlaceholder": "添加任务，回车保存（优先级中）",
  "task.addPlaceholderScoped": "添加到「{target}」，回车保存（优先级中）",
  "task.addSubtask": "添加子任务",
  "task.addSubtaskFor": "给「{title}」添加子任务",
  "task.subtaskPlaceholder": "添加子任务，回车保存",
  "task.renameHint": "双击改名",
  "task.expandSubtasks": "展开子任务",
  "task.collapseSubtasks": "收起子任务",
  "task.subtaskDoneCount.one": "子任务已完成 {done} 个，共 {total} 个",
  "task.subtaskDoneCount.other": "子任务已完成 {done} 个，共 {total} 个",
  "task.contextHint": "筛选上下文：这个父任务本身不匹配当前筛选",
  "task.completedHidden": "任务已完成，已从当前视图隐藏",
  "group.submoduleCount.one": "{count} 个子模块",
  "group.submoduleCount.other": "{count} 个子模块",

  "note.edit": "编辑备注",
  "note.aria": "备注：{summary}",
  "note.empty": "空",
  "note.title": "备注",
  "note.dialogLabel": "「{title}」",

  "menu.setStatus": "设置状态…",
  "menu.moveTo": "移动到…",
  "menu.demoteToSubtask": "降为子任务…",
  "menu.promoteToParent": "升为父任务",
  "menu.deleteTask": "删除任务",
  "menu.moveLabel": "移动到",
  "menu.demoteLabel": "降为子任务",
  "menu.moveWholeModule": "整个模块",
  "menu.noMoveTargets": "没有可移动到的位置",
  "menu.noParentTargets": "没有可挂靠的父任务",
  "menu.demoteHasChildren.one":
    "「{title}」还有 {count} 个子任务，不能直接降级；请先删除或升为父任务",
  "menu.demoteHasChildren.other":
    "「{title}」还有 {count} 个子任务，不能直接降级；请先删除或升为父任务",

  "toast.demoted": "已将「{title}」降为「{target}」的子任务",
  "toast.demotedWithPriority":
    "已将「{title}」降为「{target}」的子任务，优先级调整为{priority}",
  "toast.promoted": "已将「{title}」升为父任务",
  "toast.moved": "已移动「{title}」",
  "toast.priorityCascaded.one": "已同步下调 {count} 个子任务的优先级",
  "toast.priorityCascaded.other": "已同步下调 {count} 个子任务的优先级",
  "toast.taskDeleted": "已删除任务",
  "toast.nodeDeleted": "已删除{noun}「{label}」",
  "toast.projectDeleted": "已删除项目「{name}」",
  "toast.databaseReset": "数据库已重置",

  "confirm.markAs": "标记为{status}？",
  "confirm.deleteTask": "删除任务？",
  "confirm.deleteTaskWithChildren.one": "这个任务连同 {count} 个子任务会被删除，无法撤销。",
  "confirm.deleteTaskWithChildren.other": "这个任务连同 {count} 个子任务会被删除，无法撤销。",
  "confirm.deleteTaskAlone": "这个任务会被删除，无法撤销。",
  "confirm.deleteModule": "删除模块？",
  "confirm.deleteSubmodule": "删除子模块？",
  "confirm.deleteNodeWithTasks.one":
    "{noun}「{label}」和其中的 {count} 个任务会被删除，无法撤销。",
  "confirm.deleteNodeWithTasks.other":
    "{noun}「{label}」和其中的 {count} 个任务会被删除，无法撤销。",
  "confirm.deleteNode": "{noun}「{label}」会被删除，无法撤销。",
  "confirm.deleteProject": "删除项目？",
  "confirm.deleteProjectWithTasks.one":
    "项目「{name}」和其中的 {count} 个任务会被删除，无法撤销。",
  "confirm.deleteProjectWithTasks.other":
    "项目「{name}」和其中的 {count} 个任务会被删除，无法撤销。",
  "confirm.deleteProjectOnly": "项目「{name}」会被删除，无法撤销。",
  "confirm.resetDatabase": "重置数据库？",
  "confirm.resetDatabaseBody":
    "当前数据库文件与全部数据会被删除，随后建一个空库。此操作仅开发版本可用，无法撤销。",
  "confirm.deleteAndRebuild": "删除并重建",

  "dialog.newProject": "新建项目",
  "dialog.projectName": "项目名",
  "dialog.newModule": "新建模块",
  "dialog.moduleName": "模块名",
  "dialog.newSubmodule": "新建子模块",
  "dialog.submoduleName": "子模块名",

  "empty.todo": "「我的待办」里还没有任务。在下面输入一行，回车保存。",
  "empty.filtered": "当前筛选下没有任务。调整上方的状态或优先级标签试试。",
  "empty.scope": "{where}还没有任务。在下面输入一行，回车保存。",
  "empty.thisProject": "这个项目",
  "empty.quoted": "「{label}」",

  "statusbar.aria": "状态栏",
  "statusbar.noTasks": "暂无任务",
  "statusbar.count.one": "共 {total} 项 · 已完成 {done}",
  "statusbar.count.other": "共 {total} 项 · 已完成 {done}",

  "resize.sidebar": "调整左栏宽度",
  "window.minimize": "最小化",
  "window.maximize": "最大化",
  "window.restore": "还原",
  "window.close": "关闭",

  "about.label": "关于",
  "about.desc": "AI时代本地优先的个人桌面待办应用",
  "about.version": "版本",
  "about.platform": "平台",
  "about.runtime": "运行时",
  "about.license": "许可证",
  "about.copyright": "版权",

  "dev.reset": "重置数据库（开发）",

  "startup.documentTitle": "无法启动 — Knot",
  "startup.title": "无法启动",
  "startup.lead": "数据库迁移没有完成。程序没有继续加载，数据库文件没有被部分改写。",
  "startup.backup": "迁移前的备份：{path}",
  "startup.hint":
    "按照上面的原因处理后再打开应用。需要恢复时，用上面列出的备份文件覆盖 tasks.sqlite。",
  "startup.legacyDetail": "{detail}",

  "error.unexpected": "发生错误：{detail}",
  "error.unknown": "未知错误",
  "error.unknownBackendMessage": "未识别的后端消息（{key}）",
} as const satisfies Record<string, string>;

export type MessageKey = keyof typeof zh;

/** Base keys that have `.one` / `.other` variants. */
export type PluralBase = MessageKey extends infer Key
  ? Key extends `${infer Base}.one`
    ? Base
    : never
  : never;

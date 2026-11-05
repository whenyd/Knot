//! 命令层：薄壳。取连接 → 调 `domain::*` → 把结果交回前端。
//!
//! 这里不写业务判断，也不拼 SQL。凡是想在此处加个 `if` 的，先问它是不是规则。

use tauri::State;

use crate::db::Db;
use crate::domain::{
    self, Container, DeleteOutcome, DeletePlan, Module, MoveTarget, NodeKind, Priority, Project,
    ReparentOutcome, Status, StatusOutcome, StatusPlan, Submodule, Task, TaskCreated,
};

#[tauri::command]
pub fn list_projects(db: State<'_, Db>) -> Result<Vec<Project>, String> {
    domain::list_projects(&db.conn())
}

#[tauri::command]
pub fn create_project(db: State<'_, Db>, name: String) -> Result<Project, String> {
    domain::create_project(&db.conn(), &name)
}

#[tauri::command]
pub fn list_modules(db: State<'_, Db>, project_id: i64) -> Result<Vec<Module>, String> {
    domain::list_modules(&db.conn(), project_id)
}

#[tauri::command]
pub fn create_module(db: State<'_, Db>, project_id: i64, name: String) -> Result<Module, String> {
    domain::create_module(&db.conn(), project_id, &name)
}

#[tauri::command]
pub fn create_submodule(
    db: State<'_, Db>,
    module_id: i64,
    name: String,
) -> Result<Submodule, String> {
    domain::create_submodule(&db.conn(), module_id, &name)
}

#[tauri::command]
pub fn rename_project(db: State<'_, Db>, id: i64, name: String) -> Result<Project, String> {
    domain::rename_project(&db.conn(), id, &name)
}

#[tauri::command]
pub fn rename_module(db: State<'_, Db>, id: i64, name: String) -> Result<Module, String> {
    domain::rename_module(&db.conn(), id, &name)
}

#[tauri::command]
pub fn rename_submodule(db: State<'_, Db>, id: i64, name: String) -> Result<Submodule, String> {
    domain::rename_submodule(&db.conn(), id, &name)
}

#[tauri::command]
pub fn list_tasks(db: State<'_, Db>, container: Container) -> Result<Vec<Task>, String> {
    domain::list_tasks(&db.conn(), container)
}

#[tauri::command]
pub fn create_task(
    db: State<'_, Db>,
    container: Container,
    module_id: Option<i64>,
    submodule_id: Option<i64>,
    parent_task_id: Option<i64>,
    title: String,
) -> Result<TaskCreated, String> {
    domain::create_task(
        &db.conn(),
        container,
        module_id,
        submodule_id,
        parent_task_id,
        &title,
    )
}

#[tauri::command]
pub fn rename_task(db: State<'_, Db>, id: i64, title: String) -> Result<Task, String> {
    domain::rename_task(&db.conn(), id, &title)
}

#[tauri::command]
pub fn set_note(db: State<'_, Db>, id: i64, note: String) -> Result<Task, String> {
    domain::set_note(&db.conn(), id, &note)
}

#[tauri::command]
pub fn move_task(db: State<'_, Db>, id: i64, target: MoveTarget) -> Result<Task, String> {
    domain::move_task(&db.conn(), id, target)
}

#[tauri::command]
pub fn reparent_task(
    db: State<'_, Db>,
    id: i64,
    parent_task_id: Option<i64>,
) -> Result<ReparentOutcome, String> {
    domain::reparent_task(&db.conn(), id, parent_task_id)
}

#[tauri::command]
pub fn set_priority(db: State<'_, Db>, id: i64, priority: Priority) -> Result<Task, String> {
    domain::set_priority(&db.conn(), id, priority)
}

#[tauri::command]
pub fn plan_set_status(db: State<'_, Db>, id: i64, status: Status) -> Result<StatusPlan, String> {
    domain::plan_set_status(&db.conn(), id, status)
}

#[tauri::command]
pub fn apply_set_status(
    db: State<'_, Db>,
    id: i64,
    status: Status,
    cascade: bool,
) -> Result<StatusOutcome, String> {
    domain::apply_set_status(&db.conn(), id, status, cascade)
}

#[tauri::command]
pub fn plan_delete_task(db: State<'_, Db>, id: i64) -> Result<DeletePlan, String> {
    domain::plan_delete_task(&db.conn(), id)
}

#[tauri::command]
pub fn delete_task(db: State<'_, Db>, id: i64) -> Result<DeleteOutcome, String> {
    domain::delete_task(&db.conn(), id)
}

#[tauri::command]
pub fn plan_delete_node(db: State<'_, Db>, kind: NodeKind, id: i64) -> Result<DeletePlan, String> {
    domain::plan_delete_node(&db.conn(), kind, id)
}

#[tauri::command]
pub fn delete_node(db: State<'_, Db>, kind: NodeKind, id: i64) -> Result<(), String> {
    domain::delete_node(&db.conn(), kind, id)
}

#[tauri::command]
pub fn plan_delete_project(db: State<'_, Db>, id: i64) -> Result<DeletePlan, String> {
    domain::plan_delete_project(&db.conn(), id)
}

#[tauri::command]
pub fn delete_project(db: State<'_, Db>, id: i64) -> Result<(), String> {
    domain::delete_project(&db.conn(), id)
}

/// 仅开发版：删除活库并重建。正式版不注册这条命令。
#[cfg(debug_assertions)]
#[tauri::command]
pub fn reset_database(app: tauri::AppHandle, db: State<'_, Db>) -> Result<(), String> {
    let path = crate::db::db_path(&app)?;
    db.reset(&path)
}

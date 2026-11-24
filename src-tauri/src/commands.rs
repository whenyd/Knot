//! Command layer: a thin shell. Acquire a connection → call `domain::*` → hand the result
//! back to the frontend.
//!
//! No business decisions and no SQL here. Before adding an `if` in this file, ask whether
//! it is a rule.

use tauri::State;

use crate::db::Db;
use crate::message::Message;
use crate::domain::{
    self, Container, DeleteOutcome, DeletePlan, Module, MoveTarget, NodeKind, Priority, Project,
    ReparentOutcome, Status, StatusOutcome, StatusPlan, Submodule, Task, TaskCreated,
};

#[tauri::command]
pub fn list_projects(db: State<'_, Db>) -> Result<Vec<Project>, Message> {
    domain::list_projects(&db.conn())
}

#[tauri::command]
pub fn create_project(db: State<'_, Db>, name: String) -> Result<Project, Message> {
    domain::create_project(&db.conn(), &name)
}

#[tauri::command]
pub fn list_modules(db: State<'_, Db>, project_id: i64) -> Result<Vec<Module>, Message> {
    domain::list_modules(&db.conn(), project_id)
}

#[tauri::command]
pub fn create_module(db: State<'_, Db>, project_id: i64, name: String) -> Result<Module, Message> {
    domain::create_module(&db.conn(), project_id, &name)
}

#[tauri::command]
pub fn create_submodule(
    db: State<'_, Db>,
    module_id: i64,
    name: String,
) -> Result<Submodule, Message> {
    domain::create_submodule(&db.conn(), module_id, &name)
}

/// Reorder modules: insert before `before_id`; `None` = move to the end.
#[tauri::command]
pub fn move_module(
    db: State<'_, Db>,
    id: i64,
    before_id: Option<i64>,
) -> Result<Vec<Module>, Message> {
    domain::move_module(&db.conn(), id, before_id)
}

#[tauri::command]
pub fn rename_project(db: State<'_, Db>, id: i64, name: String) -> Result<Project, Message> {
    domain::rename_project(&db.conn(), id, &name)
}

#[tauri::command]
pub fn rename_module(db: State<'_, Db>, id: i64, name: String) -> Result<Module, Message> {
    domain::rename_module(&db.conn(), id, &name)
}

#[tauri::command]
pub fn rename_submodule(db: State<'_, Db>, id: i64, name: String) -> Result<Submodule, Message> {
    domain::rename_submodule(&db.conn(), id, &name)
}

#[tauri::command]
pub fn list_tasks(db: State<'_, Db>, container: Container) -> Result<Vec<Task>, Message> {
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
    priority: Priority,
) -> Result<TaskCreated, Message> {
    domain::create_task(
        &db.conn(),
        container,
        module_id,
        submodule_id,
        parent_task_id,
        &title,
        priority,
    )
}

#[tauri::command]
pub fn rename_task(db: State<'_, Db>, id: i64, title: String) -> Result<Task, Message> {
    domain::rename_task(&db.conn(), id, &title)
}

#[tauri::command]
pub fn set_note(db: State<'_, Db>, id: i64, note: String) -> Result<Task, Message> {
    domain::set_note(&db.conn(), id, &note)
}

#[tauri::command]
pub fn move_task(db: State<'_, Db>, id: i64, target: MoveTarget) -> Result<Task, Message> {
    domain::move_task(&db.conn(), id, target)
}

#[tauri::command]
pub fn reparent_task(
    db: State<'_, Db>,
    id: i64,
    parent_task_id: Option<i64>,
) -> Result<ReparentOutcome, Message> {
    domain::reparent_task(&db.conn(), id, parent_task_id)
}

#[tauri::command]
pub fn set_priority(db: State<'_, Db>, id: i64, priority: Priority) -> Result<Task, Message> {
    domain::set_priority(&db.conn(), id, priority)
}

#[tauri::command]
pub fn plan_set_status(db: State<'_, Db>, id: i64, status: Status) -> Result<StatusPlan, Message> {
    domain::plan_set_status(&db.conn(), id, status)
}

#[tauri::command]
pub fn apply_set_status(
    db: State<'_, Db>,
    id: i64,
    status: Status,
    cascade: bool,
) -> Result<StatusOutcome, Message> {
    domain::apply_set_status(&db.conn(), id, status, cascade)
}

#[tauri::command]
pub fn plan_delete_task(db: State<'_, Db>, id: i64) -> Result<DeletePlan, Message> {
    domain::plan_delete_task(&db.conn(), id)
}

#[tauri::command]
pub fn delete_task(db: State<'_, Db>, id: i64) -> Result<DeleteOutcome, Message> {
    domain::delete_task(&db.conn(), id)
}

#[tauri::command]
pub fn plan_delete_node(db: State<'_, Db>, kind: NodeKind, id: i64) -> Result<DeletePlan, Message> {
    domain::plan_delete_node(&db.conn(), kind, id)
}

#[tauri::command]
pub fn delete_node(db: State<'_, Db>, kind: NodeKind, id: i64) -> Result<(), Message> {
    domain::delete_node(&db.conn(), kind, id)
}

#[tauri::command]
pub fn plan_delete_project(db: State<'_, Db>, id: i64) -> Result<DeletePlan, Message> {
    domain::plan_delete_project(&db.conn(), id)
}

#[tauri::command]
pub fn delete_project(db: State<'_, Db>, id: i64) -> Result<(), Message> {
    domain::delete_project(&db.conn(), id)
}

/// Debug builds only: delete the live database and recreate it. Not registered in release builds.
#[cfg(debug_assertions)]
#[tauri::command]
pub fn reset_database(app: tauri::AppHandle, db: State<'_, Db>) -> Result<(), Message> {
    let path = crate::db::db_path(&app)?;
    db.reset(&path)
}

//! 命令层：薄壳。取连接 → 调 `domain::*` → 把结果交回前端。
//!
//! 这里不写业务判断，也不拼 SQL。凡是想在此处加个 `if` 的，先问它是不是规则。

use tauri::State;

use crate::db::Db;
use crate::domain::{self, Container, Module, Project, Task};

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
pub fn create_module(
    db: State<'_, Db>,
    project_id: i64,
    name: String,
) -> Result<Module, String> {
    domain::create_module(&db.conn(), project_id, &name)
}

#[tauri::command]
pub fn list_tasks(db: State<'_, Db>, container: Container) -> Result<Vec<Task>, String> {
    domain::list_tasks(&db.conn(), container)
}

#[tauri::command]
pub fn create_task(
    db: State<'_, Db>,
    container: Container,
    title: String,
) -> Result<Task, String> {
    domain::create_task(&db.conn(), container, &title)
}

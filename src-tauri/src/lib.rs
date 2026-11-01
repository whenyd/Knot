//! 应用装配：建连接、托管状态、注册命令。
//!
//! 启动顺序是硬的——先有可用（已迁移）的库，才开窗口；迁移失败就拒绝启动，
//! 绝不带着半迁移的库继续跑。

mod commands;
mod db;
mod domain;
mod migrations;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let database = db::Db::open_in_app_data(app.handle())?;
            app.manage(database);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_projects,
            commands::create_project,
            commands::list_modules,
            commands::create_module,
            commands::list_tasks,
            commands::create_task,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

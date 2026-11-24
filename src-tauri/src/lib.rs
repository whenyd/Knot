//! 应用装配：建连接、托管状态、注册命令。
//!
//! 启动顺序是硬的——先有可用（已迁移）的库，才让主窗口工作；迁移失败就换成
//! 一个错误窗口，绝不带着半迁移的库继续跑。

mod commands;
mod db;
mod domain;
mod migrations;

use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            match db::Db::open_in_app_data(app.handle()) {
                Ok(database) => {
                    app.manage(database);
                }
                // setup 返回 Err 会被 Tauri 直接 panic；这里自己把错误显示出来。
                Err(message) => show_startup_error(app.handle(), &message),
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_projects,
            commands::create_project,
            commands::list_modules,
            commands::create_module,
            commands::create_submodule,
            commands::move_module,
            commands::rename_project,
            commands::rename_module,
            commands::rename_submodule,
            commands::list_tasks,
            commands::create_task,
            commands::rename_task,
            commands::set_note,
            commands::move_task,
            commands::reparent_task,
            commands::set_priority,
            commands::plan_set_status,
            commands::apply_set_status,
            commands::plan_delete_task,
            commands::delete_task,
            commands::plan_delete_node,
            commands::delete_node,
            commands::plan_delete_project,
            commands::delete_project,
            #[cfg(debug_assertions)]
            commands::reset_database,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// 数据库起不来时，把主窗口换成一个独立的错误窗口。
///
/// 主窗口先藏起来（避免闪一下正常界面），错误窗口建好后再关掉主窗口；
/// 万一窗口也建不出来，至少把主窗口放回来，并把原因写到 stderr。
fn show_startup_error(app: &tauri::AppHandle, message: &str) {
    let main = app.get_webview_window("main");
    if let Some(window) = &main {
        let _ = window.hide();
    }

    let query = encode_query(message);
    let url = WebviewUrl::App(format!("index.html?startupError={query}").into());
    let built = WebviewWindowBuilder::new(app, "startup-error", url)
        .title("无法启动 — 我的待办")
        .inner_size(760.0, 520.0)
        .build();

    match built {
        Ok(_) => {
            if let Some(window) = main {
                let _ = window.close();
            }
        }
        Err(error) => {
            if let Some(window) = main {
                let _ = window.show();
            }
            eprintln!("无法显示启动错误（{error}）：{message}");
        }
    }
}

/// 逐字节 percent-encode，只保留 RFC 3986 的 unreserved 字符。
///
/// 前端用 `URLSearchParams.get` 解码；空格编码成 `%20` 而不是 `+`，
/// 中文按 UTF-8 字节展开，`&` / `#` / `%` 也不会破坏 query 结构。
fn encode_query(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(*byte as char);
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::encode_query;

    #[test]
    fn startup_error_query_encoding() {
        assert_eq!(encode_query("abc-._~019"), "abc-._~019");
        assert_eq!(
            encode_query("a b&c#d%e\n中"),
            "a%20b%26c%23d%25e%0A%E4%B8%AD"
        );
    }
}

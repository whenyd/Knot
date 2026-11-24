//! Application assembly: open the connection, manage state, register commands.
//!
//! Startup order is strict: the main window only starts working once a usable (migrated)
//! database exists. If migration fails we swap in an error window and never keep running
//! against a half-migrated database.

mod commands;
mod db;
mod domain;
mod message;
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
                // Returning Err from setup makes Tauri panic; show the error ourselves instead.
                Err(failure) => show_startup_error(app.handle(), &failure),
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

/// When the database cannot start, replace the main window with a dedicated error window.
///
/// Hide the main window first (to avoid a flash of the normal UI), then close it once the
/// error window is built. If the error window cannot be built either, at least restore the
/// main window and write the reason to stderr.
fn show_startup_error(app: &tauri::AppHandle, failure: &db::StartupFailure) {
    let main = app.get_webview_window("main");
    if let Some(window) = &main {
        let _ = window.hide();
    }

    // The window title is created before the frontend can read its saved
    // language, so it stays language-neutral; the page body is localized.
    let payload = serde_json::to_string(failure).unwrap_or_else(|_| "{}".to_string());
    let query = encode_query(&payload);
    let url = WebviewUrl::App(format!("index.html?startupError={query}").into());
    let built = WebviewWindowBuilder::new(app, "startup-error", url)
        .title("Knot")
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
            eprintln!("Could not show the startup error ({error}): {failure:?}");
        }
    }
}

/// Percent-encode byte by byte, keeping only RFC 3986 unreserved characters.
///
/// The frontend decodes with `URLSearchParams.get`; spaces become `%20` rather than `+`,
/// non-ASCII text expands as UTF-8 bytes, and `&` / `#` / `%` cannot break the query
/// structure.
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

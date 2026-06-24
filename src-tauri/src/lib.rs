mod scaffold;

use scaffold::{load_meta, run_scaffold};

#[tauri::command]
async fn path_exists(path: String) -> bool {
    tokio::fs::metadata(&path).await.is_ok()
}

/// Reveal a directory in the platform's native file manager. Plain
/// `tauri-plugin-shell::open` rejects local paths because its default scope
/// only matches URLs.
#[tauri::command]
async fn reveal_in_finder(path: String) -> Result<(), String> {
    use std::process::Command as StdCommand;
    let result = if cfg!(target_os = "macos") {
        StdCommand::new("open").arg(&path).status()
    } else if cfg!(target_os = "windows") {
        StdCommand::new("explorer").arg(&path).status()
    } else {
        StdCommand::new("xdg-open").arg(&path).status()
    };
    match result {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("打开器返回非零状态: {s}")),
        Err(e) => Err(format!("调用打开器失败: {e}")),
    }
}

/// Set the macOS Dock icon at runtime so dev mode (which doesn't run inside a
/// `.app` bundle) shows the same icon as a built bundle. Bundle builds also
/// benefit — this overrides the cached LaunchServices icon immediately.
#[cfg(target_os = "macos")]
fn apply_macos_dock_icon() {
    use objc2::ClassType;
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::{MainThreadMarker, NSData};

    const ICON_BYTES: &[u8] = include_bytes!("../icons/icon.icns");

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let ns_data = NSData::with_bytes(ICON_BYTES);
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &ns_data) else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    unsafe { app.setApplicationIconImage(Some(&image)) };
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|_app| {
            #[cfg(target_os = "macos")]
            apply_macos_dock_icon();
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load_meta,
            run_scaffold,
            path_exists,
            reveal_in_finder
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

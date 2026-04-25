use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Stdio;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// Resolve the engine root directory (yudao-scaffold/).
///
/// In dev: derived from CARGO_MANIFEST_DIR (../../yudao-scaffold).
/// In release: a sibling `yudao-scaffold/` next to the bundled app, or via
/// the YUDAO_SCAFFOLD_ENGINE env var override.
fn engine_dir(app: &AppHandle) -> PathBuf {
    if let Ok(p) = std::env::var("YUDAO_SCAFFOLD_ENGINE") {
        return PathBuf::from(p);
    }
    // Dev mode — assume the workspace layout
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidate = manifest.parent().and_then(|p| p.parent()).map(|p| p.join("yudao-scaffold"));
    if let Some(dir) = candidate {
        if dir.exists() {
            return dir;
        }
    }
    // Production fallback: look next to the executable
    if let Ok(exe) = app.path().resource_dir() {
        let p = exe.join("yudao-scaffold");
        if p.exists() {
            return p;
        }
    }
    panic!("无法定位 yudao-scaffold 引擎目录。请设置 YUDAO_SCAFFOLD_ENGINE 环境变量。")
}

/// Pick the node binary. Honors NODE_BIN, falls back to PATH lookup.
fn node_bin() -> String {
    std::env::var("NODE_BIN").unwrap_or_else(|_| "node".to_string())
}

/// Run `node --import tsx <script> [args...]` and capture stdout/stderr to a string.
async fn run_engine_script(
    app: &AppHandle,
    script_rel: &str,
    args: Vec<String>,
    stdin_payload: Option<String>,
    json_events: bool,
) -> Result<EngineResult, String> {
    let dir = engine_dir(app);
    let script = dir.join(script_rel);
    if !script.exists() {
        return Err(format!("引擎脚本不存在: {}", script.display()));
    }

    let mut cmd = Command::new(node_bin());
    cmd.current_dir(&dir)
        .arg("--import")
        .arg("tsx")
        .arg(&script)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if stdin_payload.is_some() {
        cmd.stdin(Stdio::piped());
    } else {
        cmd.stdin(Stdio::null());
    }
    if json_events {
        cmd.env("JSON_EVENTS", "1");
    }
    cmd.env("FORCE_COLOR", "0");

    let mut child = cmd.spawn().map_err(|e| format!("spawn 失败: {e}"))?;

    if let Some(payload) = stdin_payload {
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin
                .write_all(payload.as_bytes())
                .await
                .map_err(|e| format!("写 stdin 失败: {e}"))?;
            drop(stdin);
        }
    }

    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");
    let mut out_reader = BufReader::new(stdout).lines();
    let mut err_reader = BufReader::new(stderr).lines();

    let app_clone = app.clone();
    let mut full_stdout = String::new();
    let mut full_stderr = String::new();

    loop {
        tokio::select! {
            line = out_reader.next_line() => {
                match line {
                    Ok(Some(l)) => {
                        if json_events {
                            // Each event line is prefixed with the SOH byte so we can split it from
                            // human log output going to the same stdout.
                            if let Some(stripped) = l.strip_prefix('\u{0001}') {
                                if let Ok(evt) = serde_json::from_str::<serde_json::Value>(stripped) {
                                    let _ = app_clone.emit("scaffold-event", evt);
                                    continue;
                                }
                            }
                        }
                        full_stdout.push_str(&l);
                        full_stdout.push('\n');
                    }
                    Ok(None) => break,
                    Err(e) => return Err(format!("stdout 读取失败: {e}")),
                }
            }
            line = err_reader.next_line() => {
                if let Ok(Some(l)) = line {
                    full_stderr.push_str(&l);
                    full_stderr.push('\n');
                }
            }
        }
    }

    // drain any remaining stderr
    while let Ok(Some(l)) = err_reader.next_line().await {
        full_stderr.push_str(&l);
        full_stderr.push('\n');
    }

    let status = child.wait().await.map_err(|e| format!("等待子进程失败: {e}"))?;
    Ok(EngineResult {
        code: status.code().unwrap_or(-1),
        stdout: full_stdout,
        stderr: full_stderr,
    })
}

struct EngineResult {
    code: i32,
    stdout: String,
    stderr: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct RunPayload {
    answers: serde_json::Value,
    workspace: Option<String>,
    mirror: Option<String>,
    #[serde(rename = "urlOverrides", default)]
    url_overrides: serde_json::Value,
}

#[tauri::command]
async fn load_meta(
    app: AppHandle,
    workspace: Option<String>,
) -> Result<serde_json::Value, String> {
    let args = workspace.into_iter().collect::<Vec<_>>();
    let res = run_engine_script(&app, "bin/meta.ts", args, None, false).await?;
    if res.code != 0 {
        return Err(format!("meta.ts 退出码 {}: {}", res.code, res.stderr.trim()));
    }
    serde_json::from_str(&res.stdout)
        .map_err(|e| format!("解析 meta JSON 失败: {e}\n原文: {}", &res.stdout))
}

#[tauri::command]
async fn run_scaffold(app: AppHandle, payload: RunPayload) -> Result<i32, String> {
    let stdin = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
    let res = run_engine_script(&app, "bin/run.ts", vec![], Some(stdin), true).await?;
    if res.code != 0 && !res.stderr.is_empty() {
        return Err(format!("引擎退出码 {}: {}", res.code, res.stderr.trim()));
    }
    Ok(res.code)
}

/// macOS / Linux GUI apps launched from Finder/Dock get a stripped PATH
/// (`/usr/bin:/bin:/usr/sbin:/sbin`). Source the user's login shell once so
/// node installed via Homebrew / nvm / volta / asdf / fnm is reachable for
/// every subsequent `Command::new("node")`.
#[cfg(unix)]
fn hydrate_user_path() {
    let already_hydrated = std::env::var("PATH")
        .map(|p| {
            p.contains("/opt/homebrew/bin")
                || p.contains("/usr/local/bin")
                || p.contains(".nvm/")
                || p.contains(".volta/")
                || p.contains(".asdf/")
                || p.contains(".fnm/")
        })
        .unwrap_or(false);
    if already_hydrated {
        return;
    }

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let Ok(out) = std::process::Command::new(&shell)
        .args(["-l", "-c", "printf %s \"$PATH\""])
        .output()
    else {
        return;
    };
    if !out.status.success() {
        return;
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !path.is_empty() {
        std::env::set_var("PATH", path);
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

    let Some(mtm) = MainThreadMarker::new() else { return };
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
            #[cfg(unix)]
            hydrate_user_path();
            #[cfg(target_os = "macos")]
            apply_macos_dock_icon();
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![load_meta, run_scaffold])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

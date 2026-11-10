#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

/// Folders searched for helper tools (ffmpeg, deno, node). Apps launched from
/// Finder get a minimal PATH, so the usual install locations are added here.
fn tool_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = ["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"]
        .iter()
        .map(PathBuf::from)
        .collect();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(Path::new(&home).join(".deno/bin"));
        dirs.push(Path::new(&home).join(".local/bin"));
    }
    for d in ["/usr/bin", "/bin", "/usr/sbin", "/sbin"] {
        dirs.push(PathBuf::from(d));
    }
    dirs
}

fn path_env() -> String {
    tool_dirs()
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(":")
}

fn find_tool(name: &str) -> Option<PathBuf> {
    tool_dirs().into_iter().map(|d| d.join(name)).find(|p| p.is_file())
}

/// The yt-dlp binary bundled next to the app executable; falls back to one on
/// the system when running an unbundled dev build.
fn ytdlp_path() -> Result<PathBuf, String> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for name in ["yt-dlp", "yt-dlp-aarch64-apple-darwin"] {
                let p = dir.join(name);
                if p.is_file() {
                    return Ok(p);
                }
            }
        }
    }
    find_tool("yt-dlp").ok_or_else(|| "yt-dlp was not found.".to_string())
}

fn base_cmd() -> Result<Command, String> {
    let mut cmd = Command::new(ytdlp_path()?);
    cmd.env("PATH", path_env()).arg("--no-playlist");
    if let Some(ffmpeg) = find_tool("ffmpeg") {
        if let Some(dir) = ffmpeg.parent() {
            cmd.arg("--ffmpeg-location").arg(dir);
        }
    }
    // yt-dlp uses deno by default; fall back to node when deno is missing.
    if find_tool("deno").is_none() && find_tool("node").is_some() {
        cmd.args(["--js-runtimes", "node"]);
    }
    Ok(cmd)
}

fn last_error(stderr: &str) -> String {
    stderr
        .lines()
        .rev()
        .find(|l| l.starts_with("ERROR"))
        .or_else(|| stderr.lines().rev().find(|l| !l.trim().is_empty()))
        .unwrap_or("yt-dlp failed.")
        .trim_start_matches("ERROR: ")
        .to_string()
}

#[derive(Default)]
struct Downloads {
    pid: Arc<Mutex<Option<u32>>>,
    cancelled: Arc<AtomicBool>,
}

#[derive(Serialize)]
struct Env {
    ffmpeg: bool,
    js_runtime: bool,
    download_dir: String,
}

#[tauri::command]
fn env_status(app: AppHandle) -> Env {
    let download_dir = app
        .path()
        .download_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    Env {
        ffmpeg: find_tool("ffmpeg").is_some(),
        js_runtime: find_tool("deno").is_some() || find_tool("node").is_some(),
        download_dir,
    }
}

/// Returns yt-dlp's JSON description of the video (title, formats, ...).
#[tauri::command]
async fn fetch_info(url: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let out = base_cmd()?
            .args(["-J", "--no-warnings", "--", &url])
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("Could not run yt-dlp: {e}"))?;
        if !out.status.success() {
            return Err(last_error(&String::from_utf8_lossy(&out.stderr)));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Serialize, Clone)]
struct Progress {
    downloaded: f64,
    total: f64,
    speed: f64,
    eta: f64,
}

#[derive(Serialize, Clone)]
struct Done {
    ok: bool,
    cancelled: bool,
    path: String,
    error: String,
}

#[tauri::command]
fn start_download(
    app: AppHandle,
    state: State<Downloads>,
    url: String,
    format: String,
    merge: String,
    audio_convert: String,
    label: String,
    out_dir: String,
) -> Result<(), String> {
    if state.pid.lock().unwrap().is_some() {
        return Err("A download is already running.".into());
    }
    let label: String = label.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let template = if label.is_empty() {
        "%(title).120B.%(ext)s".to_string()
    } else {
        format!("%(title).120B [{label}].%(ext)s")
    };

    let mut cmd = base_cmd()?;
    cmd.args(["--newline", "--progress", "--no-mtime", "--force-overwrites"])
        .args(["-f", &format])
        .args(["-P", &out_dir])
        .args(["-o", &template])
        .args([
            "--progress-template",
            "download:PROG|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.total_bytes_estimate)s|%(progress.speed)s|%(progress.eta)s",
        ])
        .args(["--print", "after_move:DONE|%(filepath)s"]);
    if !merge.is_empty() {
        cmd.args(["--merge-output-format", &merge]);
    }
    if !audio_convert.is_empty() {
        cmd.args(["-x", "--audio-format", &audio_convert, "--audio-quality", "0"]);
    }
    cmd.arg("--").arg(&url);

    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not run yt-dlp: {e}"))?;

    state.cancelled.store(false, Ordering::SeqCst);
    *state.pid.lock().unwrap() = Some(child.id());
    let pid_slot = state.pid.clone();
    let cancelled = state.cancelled.clone();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    let err_thread = std::thread::spawn(move || {
        let mut text = String::new();
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            text.push_str(&line);
            text.push('\n');
        }
        text
    });

    std::thread::spawn(move || {
        let mut path = String::new();
        let num = |s: &str| s.parse::<f64>().unwrap_or(0.0);
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(rest) = line.strip_prefix("PROG|") {
                let p: Vec<&str> = rest.split('|').collect();
                if p.len() == 5 {
                    let total = if num(p[1]) > 0.0 { num(p[1]) } else { num(p[2]) };
                    let _ = app.emit(
                        "dl-progress",
                        Progress { downloaded: num(p[0]), total, speed: num(p[3]), eta: num(p[4]) },
                    );
                }
            } else if let Some(rest) = line.strip_prefix("DONE|") {
                path = rest.to_string();
            }
        }
        let ok = child.wait().map(|s| s.success()).unwrap_or(false);
        let stderr_text = err_thread.join().unwrap_or_default();
        *pid_slot.lock().unwrap() = None;
        let was_cancelled = cancelled.load(Ordering::SeqCst);
        let _ = app.emit(
            "dl-done",
            Done {
                ok: ok && !path.is_empty(),
                cancelled: was_cancelled,
                path,
                error: if ok { String::new() } else { last_error(&stderr_text) },
            },
        );
    });
    Ok(())
}

#[tauri::command]
fn cancel_download(state: State<Downloads>) {
    if let Some(pid) = *state.pid.lock().unwrap() {
        state.cancelled.store(true, Ordering::SeqCst);
        // SIGINT lets yt-dlp shut down its own child processes (ffmpeg) cleanly.
        let _ = Command::new("/bin/kill").args(["-INT", &pid.to_string()]).status();
    }
}

#[tauri::command]
async fn pick_folder(app: AppHandle) -> Option<String> {
    app.dialog().file().blocking_pick_folder().map(|p| p.to_string())
}

/// Reads the clipboard natively; the webview's own clipboard API is blocked
/// behind a permission prompt.
#[tauri::command]
fn read_clipboard() -> String {
    Command::new("/usr/bin/pbpaste")
        .env("LANG", "en_US.UTF-8")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

#[tauri::command]
fn reveal(path: String) {
    let _ = Command::new("/usr/bin/open").arg("-R").arg(path).spawn();
}

#[tauri::command]
fn open_file(path: String) {
    let _ = Command::new("/usr/bin/open").arg(path).spawn();
}

#[cfg(feature = "selftest")]
#[tauri::command]
fn selftest_report(app: AppHandle, msg: String) {
    println!("SELFTEST {msg}");
    if msg.starts_with("END") {
        app.exit(0);
    }
}

#[cfg(not(feature = "selftest"))]
#[tauri::command]
fn selftest_report() {}

/// Hidden end-to-end check used only by `--features selftest` builds: drives
/// the real UI code through a fetch and a download without showing a window.
#[cfg(feature = "selftest")]
fn selftest(app: &mut tauri::App) {
    let (Ok(url), Ok(dir)) = (std::env::var("TUBEGRAB_SELFTEST_URL"), std::env::var("TUBEGRAB_SELFTEST_DIR")) else {
        return;
    };
    app.set_activation_policy(tauri::ActivationPolicy::Accessory);
    let window = app.get_webview_window("main").unwrap();
    // A hidden window does not run its JavaScript, so park it off-screen instead.
    let _ = window.set_position(tauri::LogicalPosition::new(-5000.0, -5000.0));
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(3));
        let js = format!(
            r#"(async () => {{
              const say = (m) => invoke("selftest_report", {{ msg: m }});
              try {{
                let seen = 0;
                await listen("dl-progress", () => seen++);
                await listen("dl-done", (e) => say("END " + JSON.stringify(e.payload) + " progressEvents=" + seen + " status=" + $("status").textContent));
                $("url").value = {url:?};
                await fetchInfo();
                if (!info) return say("END fetch failed: " + $("error").textContent);
                say("fetched title=" + info.title + " tabs=" + groups.map((g) => g.name + ":" + g.options.length).join(", ") + " selected=" + selected.quality + " " + selected.format);
                outDir = {dir:?};
                await startDownload();
              }} catch (e) {{ say("END exception " + e); }}
            }})();"#
        );
        let _ = window.eval(&js);
    });
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Downloads::default())
        .setup(|_app| {
            #[cfg(feature = "selftest")]
            selftest(_app);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            env_status,
            fetch_info,
            start_download,
            cancel_download,
            pick_folder,
            reveal,
            open_file,
            read_clipboard,
            selftest_report
        ])
        .run(tauri::generate_context!())
        .expect("error while running TubeGrab");
}

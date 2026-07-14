#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

mod audio;
mod ui_prefs;
mod updater;
mod usage_refresher;
mod watcher;

use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::menu::{CheckMenuItem, ContextMenu, Menu, MenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Listener, Manager};

/// Tray "Opacity" choices. Ids look like "opacity-70".
const OPACITY_STEPS: [(&str, f64); 4] =
    [("opacity-100", 1.0), ("opacity-85", 0.85), ("opacity-70", 0.7), ("opacity-55", 0.55)];

/// Windows fires Moved continuously during a drag (one event per pixel);
/// saving ui.json on each would be a read+write+rename storm that also wakes
/// the ~/.clawdometer watcher every time. Coalesce to one save per drag.
static MOVE_DEBOUNCE: Mutex<ui_prefs::MoveDebouncer> = Mutex::new(ui_prefs::MoveDebouncer::new());
const MOVE_SETTLE: Duration = Duration::from_millis(500);

fn save_position(app: &tauri::AppHandle, x: i32, y: i32) {
    let mut p = current_prefs(app);
    p.x = x;
    p.y = y;
    ui_prefs::save(&ui_path(), &p);
}

/// Persist any not-yet-settled drag position immediately (quit path — the
/// flusher thread dies with the process).
fn flush_pending_move(app: &tauri::AppHandle) {
    let pos = MOVE_DEBOUNCE.lock().ok().and_then(|mut d| d.take_now());
    if let Some((x, y)) = pos {
        save_position(app, x, y);
    }
}

fn ui_path() -> std::path::PathBuf {
    clawdometer_core::paths::clawdometer_dir().join("ui.json")
}

/// Saved prefs, or defaults seeded with the window's current position so a
/// first-ever tray change doesn't pin the HUD to (0,0) on next start.
fn current_prefs(app: &tauri::AppHandle) -> ui_prefs::UiPrefs {
    ui_prefs::load(&ui_path()).unwrap_or_else(|| {
        let pos = app
            .get_webview_window("hud")
            .and_then(|w| w.outer_position().ok())
            .unwrap_or(tauri::PhysicalPosition::new(0, 0));
        ui_prefs::UiPrefs {
            x: pos.x,
            y: pos.y,
            opacity: 1.0,
            compact: false,
            rice: ui_prefs::default_rice(),
        }
    })
}

/// Push opacity/compact to the window (size) and the webview (CSS). Called on
/// webview ready and after every tray change.
fn apply_prefs(app: &tauri::AppHandle, prefs: &ui_prefs::UiPrefs) {
    if let Some(win) = app.get_webview_window("hud") {
        // The Audiowave Orb skin is a square ring stage; Classic keeps its
        // regular or compact card size.
        let (w, h) = if prefs.rice.starts_with("audiowave_orb") {
            (160.0, 160.0)
        } else if prefs.compact {
            (120.0, 92.0)
        } else {
            (200.0, 112.0)
        };
        let _ = win.set_size(tauri::LogicalSize::new(w, h));
    }
    let _ = app.emit(
        "ui-prefs",
        serde_json::json!({
            "opacity": prefs.opacity,
            "compact": prefs.compact,
            "rice": prefs.rice,
        }),
    );
}

fn toggle_hud(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("hud") {
        let visible = win.is_visible().unwrap_or(false);
        if visible {
            let _ = win.hide();
        } else {
            let _ = win.show();
            let _ = win.set_focus();
        }
    }
}

/// Statusline auto-install so the HUD works standalone (no CLI download):
/// with no live poller, Claude Code's statusline hook is the only data
/// source. Conservative about consent — install only when the `statusLine`
/// key is free to claim or already a clawdometer hook (repairing a stale exe
/// path is idempotent). A user's own statusline is never touched: chaining it
/// stays an explicit `clawdometer install` (CLI) decision. The marker makes
/// the no-key claim a one-time offer, so removing our key afterwards sticks.
fn should_autoinstall(settings_path: &std::path::Path, marker: &std::path::Path) -> bool {
    let root = match std::fs::read_to_string(settings_path) {
        Ok(raw) => {
            match serde_json::from_str::<serde_json::Value>(raw.trim_start_matches('\u{feff}')) {
                Ok(v) => v,
                Err(_) => return false, // malformed settings: never auto-edit
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(_) => return false,
    };
    match root.get(clawdometer_core::settings::STATUSLINE_KEY) {
        Some(sl) => sl
            .get("command")
            .and_then(|c| c.as_str())
            .map(clawdometer_core::settings::is_clawdometer_hook_command)
            .unwrap_or(false),
        None => !marker.exists(),
    }
}

fn autoinstall_statusline() {
    let settings = clawdometer_core::paths::default_claude_settings_path();
    let claw = clawdometer_core::paths::clawdometer_dir();
    let marker = claw.join("statusline-autoinstall.done");
    if !should_autoinstall(&settings, &marker) {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let command = format!("\"{}\" hook", exe.display());
    let fmt = time::format_description::parse_borrowed::<2>(
        "[year][month][day]-[hour][minute][second]",
    )
    .expect("static format");
    let timestamp = time::OffsetDateTime::now_utc()
        .format(&fmt)
        .unwrap_or_else(|_| "unknown".into());
    // Best-effort: same backup + wrap-safety as the CLI's install. Failure
    // just means the HUD waits for data until the user runs the CLI install.
    let _ = clawdometer_core::settings::install(&settings, &claw, &command, &timestamp);
    let _ = std::fs::create_dir_all(&claw);
    let _ = std::fs::write(&marker, b"");
}

fn main() {
    // `clawdometer-app.exe hook` — the same statusline hook the CLI exposes,
    // so the HUD binary alone can serve as the statusline command it
    // auto-installs. Must run before any Tauri init: a hook invocation is a
    // short-lived child of Claude Code, not a second HUD (the single-instance
    // plugin would otherwise front the running HUD and exit non-zero).
    if std::env::args().nth(1).as_deref() == Some("hook") {
        let line = std::panic::catch_unwind(clawdometer_core::hook::run_hook)
            .unwrap_or_else(|_| String::from("clawdometer"));
        use std::io::Write as _;
        let _ = writeln!(std::io::stdout(), "{line}");
        return;
    }
    // Give this windowless process a hidden console before anything spawns the
    // headless `claude -p /usage` refresh — claude only prints its rate-limit
    // numbers when a console is attached. Must come after the `hook` return
    // above (that path's stdout belongs to Claude Code).
    usage_refresher::ensure_hidden_console();
    autoinstall_statusline();
    tauri::Builder::default()
        // Must be the first registered plugin (per its docs). A second launch
        // (autostart already running + manual start) would otherwise mean two
        // tray icons and two HUDs racing on ui.json — instead the new process
        // exits and the existing HUD is shown.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(win) = app.get_webview_window("hud") {
                let _ = win.show();
                let _ = win.set_focus();
            }
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let prefs = current_prefs(app.handle());
            let show_hide = MenuItem::with_id(app, "toggle", "Show/Hide", true, None::<&str>)?;
            let refresh =
                MenuItem::with_id(app, "refresh-usage", "Refresh usage", true, None::<&str>)?;
            let compact_item = CheckMenuItem::with_id(
                app, "compact", "Compact size", true, prefs.compact, None::<&str>,
            )?;
            let opacity_items = OPACITY_STEPS
                .iter()
                .map(|(id, val)| {
                    CheckMenuItem::with_id(
                        app,
                        *id,
                        id.trim_start_matches("opacity-").to_string() + "%",
                        true,
                        (prefs.opacity - val).abs() < 0.01,
                        None::<&str>,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            let opacity_menu = Submenu::with_items(
                app,
                "Opacity",
                true,
                &opacity_items.iter().map(|i| i as &dyn tauri::menu::IsMenuItem<_>).collect::<Vec<_>>(),
            )?;
            // RICE skin profiles. Classic sits at the top level; the two
            // Audiowave Orb variants — "Bars" (rings only) and "Peak hold"
            // (bars + falling peak caps) — nest under their own arrow submenu.
            // All three are one radio group: the handler keeps exactly one
            // checked across the whole set. Both orb ids share the
            // "audiowave_orb" prefix so window sizing / audio capture treat
            // them alike.
            let rice_classic = CheckMenuItem::with_id(
                app, "rice-classic", "Classic", true, prefs.rice == "classic", None::<&str>,
            )?;
            let orb_bars = CheckMenuItem::with_id(
                app, "rice-audiowave_orb", "Bars", true, prefs.rice == "audiowave_orb", None::<&str>,
            )?;
            let orb_peak = CheckMenuItem::with_id(
                app, "rice-audiowave_orb_peak", "Peak hold", true,
                prefs.rice == "audiowave_orb_peak", None::<&str>,
            )?;
            let orb_menu =
                Submenu::with_items(app, "Audiowave Orb", true, &[&orb_bars, &orb_peak])?;
            let rice_menu = Submenu::with_items(
                app,
                "RICE",
                true,
                &[&rice_classic as &dyn tauri::menu::IsMenuItem<_>, &orb_menu],
            )?;
            // The radio set the menu handler syncs when any rice id is picked.
            let rice_items = vec![rice_classic, orb_bars, orb_peak];
            // Seed the checkmark from the actual Run-key state so the menu
            // shows whether autostart is on instead of flipping blind.
            let autostart_enabled = {
                use tauri_plugin_autostart::ManagerExt;
                app.autolaunch().is_enabled().unwrap_or(false)
            };
            let autostart = CheckMenuItem::with_id(
                app, "autostart", "Start with Windows", true, autostart_enabled, None::<&str>,
            )?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[&show_hide, &refresh, &compact_item, &opacity_menu, &rice_menu, &autostart, &quit],
            )?;
            // Cloned for the HUD right-click, which pops this same full menu.
            let context_menu = menu.clone();

            TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().expect("bundled icon").clone())
                .tooltip("Clawdometer")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| {
                    // Click fires for both Down and Up; toggle once per click.
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        toggle_hud(tray.app_handle());
                    }
                })
                .on_menu_event({
                    let compact_item = compact_item.clone();
                    let opacity_items = opacity_items.clone();
                    let autostart_item = autostart.clone();
                    let rice_items = rice_items.clone();
                    move |app, event| match event.id().as_ref() {
                        "toggle" => toggle_hud(app),
                        "refresh-usage" => usage_refresher::request_refresh(),
                        // CheckMenuItem toggles itself on click; read the new
                        // state back rather than flipping blind.
                        "compact" => {
                            let mut p = current_prefs(app);
                            p.compact = compact_item.is_checked().unwrap_or(!p.compact);
                            ui_prefs::save(&ui_path(), &p);
                            apply_prefs(app, &p);
                        }
                        id if id.starts_with("opacity-") => {
                            let Some((_, val)) =
                                OPACITY_STEPS.iter().find(|(step_id, _)| *step_id == id)
                            else {
                                return;
                            };
                            let mut p = current_prefs(app);
                            p.opacity = *val;
                            ui_prefs::save(&ui_path(), &p);
                            apply_prefs(app, &p);
                            // Radio behavior: exactly the picked step stays checked.
                            for item in &opacity_items {
                                let _ = item.set_checked(item.id().as_ref() == id);
                            }
                        }
                        id if id.starts_with("rice-") => {
                            let profile = id.trim_start_matches("rice-").to_string();
                            let mut p = current_prefs(app);
                            p.rice = profile.clone();
                            ui_prefs::save(&ui_path(), &p);
                            // Resizes the window (orb = square) and emits the
                            // skin change to the webview.
                            apply_prefs(app, &p);
                            // Radio behavior: only the picked profile stays checked.
                            for item in &rice_items {
                                let _ = item.set_checked(item.id().as_ref() == id);
                            }
                            // Loopback capture runs for either orb variant, so
                            // Classic costs nothing.
                            audio::set_active(app, profile.starts_with("audiowave_orb"));
                        }
                        "autostart" => {
                            // Explicit user action — the one write outside ~/.clawdometer
                            // (HKCU Run key), documented in the README. CheckMenuItem
                            // toggles itself on click; read the new state back rather
                            // than flipping blind (same pattern as "compact").
                            use tauri_plugin_autostart::ManagerExt;
                            let mgr = app.autolaunch();
                            let want = autostart_item
                                .is_checked()
                                .unwrap_or(!mgr.is_enabled().unwrap_or(false));
                            let _ = if want { mgr.enable() } else { mgr.disable() };
                            // Re-sync the checkmark with what actually happened
                            // (enable/disable can fail).
                            let _ = autostart_item.set_checked(mgr.is_enabled().unwrap_or(want));
                        }
                        "quit" => {
                            flush_pending_move(app);
                            app.exit(0);
                        }
                        _ => {}
                    }
                })
                .build(app)?;
            // Restore HUD position. Skip Windows' minimized-window sentinel
            // (-32000, -32000) and positions outside every current monitor
            // (e.g. a since-unplugged display) — restoring to either would
            // leave the HUD permanently off-screen.
            if let (Some(win), Some(prefs)) =
                (app.get_webview_window("hud"), ui_prefs::load(&ui_path()))
            {
                // Require at least MIN_VISIBLE px of the window on some
                // monitor — a bare top-left containment check accepts a
                // position 1 px inside a monitor's bottom-right corner,
                // leaving the HUD effectively off-screen.
                const MIN_VISIBLE: i32 = 40;
                let (w, h) = win
                    .outer_size()
                    .map(|s| (s.width as i32, s.height as i32))
                    .unwrap_or((200, 112));
                let on_a_monitor = win
                    .available_monitors()
                    .map(|monitors| {
                        monitors.iter().any(|m| {
                            let p = m.position();
                            let s = m.size();
                            let vis_w =
                                (prefs.x + w).min(p.x + s.width as i32) - prefs.x.max(p.x);
                            let vis_h =
                                (prefs.y + h).min(p.y + s.height as i32) - prefs.y.max(p.y);
                            vis_w >= MIN_VISIBLE.min(w) && vis_h >= MIN_VISIBLE.min(h)
                        })
                    })
                    .unwrap_or(true);
                if prefs.x > -30000 && prefs.y > -30000 && on_a_monitor {
                    let _ = win.set_position(tauri::PhysicalPosition::new(prefs.x, prefs.y));
                }
            }
            // Apply saved size now (before the webview paints) and re-send
            // opacity/compact once the webview signals it's listening —
            // events emitted before the listener attaches are lost.
            apply_prefs(app.handle(), &prefs);
            // Start loopback capture now if the saved skin is the orb, so it
            // reacts the moment the HUD paints.
            if prefs.rice.starts_with("audiowave_orb") {
                audio::set_active(app.handle(), true);
            }
            let handle = app.handle().clone();
            app.listen("ui-ready", move |_| {
                let p = current_prefs(&handle);
                apply_prefs(&handle, &p);
            });
            // Right-clicking the HUD pops the full settings menu at the cursor
            // (JS suppresses WebView2's own menu and emits "hud-context"), so the
            // panel offers the same items as the tray — Show/Hide, Refresh,
            // Compact, Opacity, RICE, autostart, Quit. Items route through the
            // same on_menu_event handler, so a pick saves + applies + syncs the
            // checkmarks with no extra logic.
            let popup_handle = app.handle().clone();
            let popup_menu = context_menu.clone();
            app.listen("hud-context", move |_| {
                if let Some(win) = popup_handle.get_webview_window("hud") {
                    let _ = popup_menu.popup(win.as_ref().window());
                }
            });
            // Double-clicking the HUD toggles compact size — same effect as the
            // tray's "Compact size" item, kept in sync (window resize + CSS via
            // apply_prefs, and the tray checkmark).
            let compact_handle = app.handle().clone();
            let compact_toggle = compact_item.clone();
            app.listen("toggle-compact", move |_| {
                let mut p = current_prefs(&compact_handle);
                p.compact = !p.compact;
                ui_prefs::save(&ui_path(), &p);
                apply_prefs(&compact_handle, &p);
                let _ = compact_toggle.set_checked(p.compact);
            });
            // Leftover from versions whose OAuth-token poller wrote this file
            // (the poller was removed — Anthropic's Consumer ToS prohibits
            // reusing Claude Code's OAuth token in third-party tools).
            let _ = std::fs::remove_file(
                clawdometer_core::paths::clawdometer_dir().join("poll_error.json"),
            );
            watcher::spawn(app.handle().clone());
            usage_refresher::spawn();
            // Ask GitHub for a newer release and, on the user's OK, install it
            // in the background. Silent when offline or already current.
            updater::check_on_startup(app.handle());
            // Flush the debounced drag position once the window settles.
            let flush_handle = app.handle().clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_millis(250));
                let pos = MOVE_DEBOUNCE
                    .lock()
                    .ok()
                    .and_then(|mut d| d.take_if_settled(Instant::now(), MOVE_SETTLE));
                if let Some((x, y)) = pos {
                    save_position(&flush_handle, x, y);
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Moved(pos) = event {
                // The -30000 threshold catches Windows' minimized-park
                // sentinel (-32000 physical). Belt-and-braces: also skip while
                // minimized, in case a Windows version/DPI combo ever parks
                // inside the threshold (a fresh launch has been observed
                // parked-minimized in the wild, 2026-07-12).
                if window.label() == "hud"
                    && pos.x > -30000
                    && pos.y > -30000
                    && !window.is_minimized().unwrap_or(false)
                {
                    // Record only — the flusher thread saves once the drag
                    // settles (position-only update; opacity/compact intact).
                    if let Ok(mut d) = MOVE_DEBOUNCE.lock() {
                        d.record(pos.x, pos.y, Instant::now());
                    }
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("failed to run clawdometer app");
}

#[cfg(test)]
mod tests {
    use super::should_autoinstall;

    fn dirs() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        let marker = dir.path().join("statusline-autoinstall.done");
        (dir, settings, marker)
    }

    #[test]
    fn claims_missing_settings_or_missing_key_once() {
        let (_d, settings, marker) = dirs();
        assert!(should_autoinstall(&settings, &marker), "no settings file: claim");
        std::fs::write(&settings, r#"{"model": "opus"}"#).unwrap();
        assert!(should_autoinstall(&settings, &marker), "no statusLine key: claim");
        std::fs::write(&marker, b"").unwrap();
        assert!(!should_autoinstall(&settings, &marker), "already offered once: respect removal");
    }

    #[test]
    fn repairs_own_hook_even_after_marker_but_never_touches_foreign_statusline() {
        let (_d, settings, marker) = dirs();
        std::fs::write(&marker, b"").unwrap();
        std::fs::write(
            &settings,
            r#"{"statusLine": {"command": "\"C:\\old\\clawdometer-app.exe\" hook"}}"#,
        )
        .unwrap();
        assert!(should_autoinstall(&settings, &marker), "own stale hook: repair");
        std::fs::write(&settings, r#"{"statusLine": {"command": "my-own-line.cmd"}}"#).unwrap();
        assert!(!should_autoinstall(&settings, &marker), "user's statusline: hands off");
    }

    #[test]
    fn never_edits_malformed_settings() {
        let (_d, settings, marker) = dirs();
        std::fs::write(&settings, "{ torn").unwrap();
        assert!(!should_autoinstall(&settings, &marker));
    }
}

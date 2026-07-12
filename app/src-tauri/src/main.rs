#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

mod ui_prefs;
mod usage_poller;
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
    ui_prefs::save(&ui_path(), p);
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
        ui_prefs::UiPrefs { x: pos.x, y: pos.y, opacity: 1.0, compact: false }
    })
}

/// Push opacity/compact to the window (size) and the webview (CSS). Called on
/// webview ready and after every tray change.
fn apply_prefs(app: &tauri::AppHandle, prefs: &ui_prefs::UiPrefs) {
    if let Some(win) = app.get_webview_window("hud") {
        let (w, h) = if prefs.compact { (120.0, 92.0) } else { (200.0, 112.0) };
        let _ = win.set_size(tauri::LogicalSize::new(w, h));
    }
    let _ = app.emit(
        "ui-prefs",
        serde_json::json!({ "opacity": prefs.opacity, "compact": prefs.compact }),
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

fn main() {
    tauri::Builder::default()
        // Must be the first registered plugin (per its docs). A second launch
        // (autostart already running + manual start) would otherwise mean two
        // tray icons, two HUDs, and two pollers racing on live.json/ui.json —
        // instead the new process exits and the existing HUD is shown.
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
        .setup(|app| {
            let prefs = current_prefs(app.handle());
            let show_hide = MenuItem::with_id(app, "toggle", "Show/Hide", true, None::<&str>)?;
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
                &[&show_hide, &compact_item, &opacity_menu, &autostart, &quit],
            )?;

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
                    move |app, event| match event.id().as_ref() {
                        "toggle" => toggle_hud(app),
                        // CheckMenuItem toggles itself on click; read the new
                        // state back rather than flipping blind.
                        "compact" => {
                            let mut p = current_prefs(app);
                            p.compact = compact_item.is_checked().unwrap_or(!p.compact);
                            ui_prefs::save(&ui_path(), p);
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
                            ui_prefs::save(&ui_path(), p);
                            apply_prefs(app, &p);
                            // Radio behavior: exactly the picked step stays checked.
                            for item in &opacity_items {
                                let _ = item.set_checked(item.id().as_ref() == id);
                            }
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
                let on_a_monitor = win
                    .available_monitors()
                    .map(|monitors| {
                        monitors.iter().any(|m| {
                            let p = m.position();
                            let s = m.size();
                            prefs.x >= p.x
                                && prefs.x < p.x + s.width as i32
                                && prefs.y >= p.y
                                && prefs.y < p.y + s.height as i32
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
            let handle = app.handle().clone();
            app.listen("ui-ready", move |_| {
                let p = current_prefs(&handle);
                apply_prefs(&handle, &p);
            });
            // Right-clicking the HUD pops the native Opacity menu at the cursor
            // (JS suppresses WebView2's own menu and emits "hud-context"). Items
            // route through the same on_menu_event handler as the tray, so a
            // pick saves + applies + syncs the tray checkmarks — no new logic.
            let popup_handle = app.handle().clone();
            let popup_menu = opacity_menu.clone();
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
                ui_prefs::save(&ui_path(), p);
                apply_prefs(&compact_handle, &p);
                let _ = compact_toggle.set_checked(p.compact);
            });
            watcher::spawn(app.handle().clone());
            usage_poller::spawn();
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

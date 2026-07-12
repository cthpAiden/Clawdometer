#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

mod ui_prefs;
mod usage_poller;
mod watcher;

use tauri::menu::{CheckMenuItem, Menu, MenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Listener, Manager};

/// Tray "Opacity" choices. Ids look like "opacity-70".
const OPACITY_STEPS: [(&str, f64); 4] =
    [("opacity-100", 1.0), ("opacity-85", 0.85), ("opacity-70", 0.7), ("opacity-55", 0.55)];

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
        let (w, h) = if prefs.compact { (150.0, 84.0) } else { (260.0, 120.0) };
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
            let autostart = MenuItem::with_id(app, "autostart", "Start with Windows", true, None::<&str>)?;
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
                            // (HKCU Run key), documented in the README.
                            use tauri_plugin_autostart::ManagerExt;
                            let mgr = app.autolaunch();
                            let enabled = mgr.is_enabled().unwrap_or(false);
                            let _ = if enabled { mgr.disable() } else { mgr.enable() };
                        }
                        "quit" => app.exit(0),
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
            watcher::spawn(app.handle().clone());
            usage_poller::spawn();
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Moved(pos) = event {
                if window.label() == "hud" && pos.x > -30000 && pos.y > -30000 {
                    // Update position only — keep opacity/compact intact.
                    let mut p = current_prefs(window.app_handle());
                    p.x = pos.x;
                    p.y = pos.y;
                    ui_prefs::save(&ui_path(), p);
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("failed to run clawdometer app");
}

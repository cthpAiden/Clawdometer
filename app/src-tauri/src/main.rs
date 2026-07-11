#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

mod ui_prefs;
mod watcher;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::Manager;

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
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            let show_hide = MenuItem::with_id(app, "toggle", "Show/Hide", true, None::<&str>)?;
            let autostart = MenuItem::with_id(app, "autostart", "Start with Windows", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_hide, &autostart, &quit])?;

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
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "toggle" => toggle_hud(app),
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
                })
                .build(app)?;
            // Restore HUD position. Skip Windows' minimized-window sentinel
            // (-32000, -32000) — restoring to it would leave the HUD
            // permanently off-screen.
            let ui_path = clawdometer_core::paths::clawdometer_dir().join("ui.json");
            if let (Some(win), Some(prefs)) =
                (app.get_webview_window("hud"), ui_prefs::load(&ui_path))
            {
                if prefs.x > -30000 && prefs.y > -30000 {
                    let _ = win.set_position(tauri::PhysicalPosition::new(prefs.x, prefs.y));
                }
            }
            watcher::spawn(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Moved(pos) = event {
                if window.label() == "hud" && pos.x > -30000 && pos.y > -30000 {
                    let ui_path = clawdometer_core::paths::clawdometer_dir().join("ui.json");
                    ui_prefs::save(&ui_path, ui_prefs::UiPrefs { x: pos.x, y: pos.y });
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("failed to run clawdometer app");
}

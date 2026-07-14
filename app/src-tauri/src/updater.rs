// Auto-update: on startup, ask GitHub for a newer release, prompt the user, and
// (on yes) download + install it in the background via the NSIS "passive"
// installer — a progress bar, no wizard clicks. The whole flow is Rust-side so
// the 200x112 HUD never has to host a dialog; failures (offline, no published
// release, draft-only release) stay silent so a normal launch is never blocked.

use tauri::AppHandle;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
use tauri_plugin_updater::UpdaterExt;

pub fn check_on_startup(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        // No updater configured / build without a pubkey: nothing to do.
        let Ok(updater) = handle.updater() else { return };
        // check() is a network call; offline or a parse error just means "not
        // now" — never surface it, the HUD works fine without updating.
        let update = match updater.check().await {
            Ok(Some(u)) => u,
            _ => return,
        };
        let msg = format!(
            "Clawdometer {} is available (you have {}). Install it now?",
            update.version, update.current_version
        );
        // Callback form, not blocking_show(): this HUD autostarts at login, so
        // the dialog can sit unattended for days — a blocking wait would park a
        // tokio worker thread that whole time. The dialog runs on its own
        // thread; the closure fires with true only for the "Update" button.
        handle
            .dialog()
            .message(msg)
            .title("Clawdometer update")
            .buttons(MessageDialogButtons::OkCancelCustom(
                "Update".into(),
                "Later".into(),
            ))
            .show(move |yes| {
                if !yes {
                    return;
                }
                tauri::async_runtime::spawn(async move {
                    // Downloads then launches the NSIS installer in passive
                    // mode (/P /R). On Windows this never returns: the plugin
                    // exits the process and the installer relaunches the
                    // updated exe itself once done. On failure (bad signature,
                    // network drop) we just stay on the current version; the
                    // prompt reappears next launch.
                    let _ = update.download_and_install(|_, _| {}, || {}).await;
                });
            });
    });
}

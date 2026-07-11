use std::path::PathBuf;

use clawdometer_core::paths;
use clawdometer_core::settings::{
    install, uninstall, InstallOutcome, UninstallOutcome,
};
use clawdometer_core::state::{read_state, render_statusline};

/// `"<absolute exe path>" hook` — quoted because install paths contain spaces.
fn our_command() -> String {
    let exe = std::env::current_exe()
        .unwrap_or_else(|_| PathBuf::from("clawdometer.exe"));
    format!("\"{}\" hook", exe.display())
}

fn settings_path(args: &[String]) -> PathBuf {
    args.iter()
        .position(|a| a == "--settings")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(paths::default_claude_settings_path)
}

fn backup_timestamp() -> String {
    let fmt = time::format_description::parse("[year][month][day]-[hour][minute][second]")
        .expect("static format");
    time::OffsetDateTime::now_utc()
        .format(&fmt)
        .unwrap_or_else(|_| "unknown".into())
}

pub fn cmd_install(args: &[String]) -> i32 {
    let sp = settings_path(args);
    let claw = paths::clawdometer_dir();
    match install(&sp, &claw, &our_command(), &backup_timestamp()) {
        Ok(InstallOutcome::Installed) => {
            println!("installed: statusLine set in {}", sp.display());
            0
        }
        Ok(InstallOutcome::Wrapped) => {
            println!(
                "installed: previous statusLine preserved in {} and will be chained",
                claw.join("wrapped.json").display()
            );
            0
        }
        Ok(InstallOutcome::AlreadyInstalled) => {
            println!("already installed — nothing to do");
            0
        }
        Err(e) => {
            eprintln!("install aborted: {e}");
            1
        }
    }
}

pub fn cmd_uninstall(args: &[String]) -> i32 {
    let sp = settings_path(args);
    let claw = paths::clawdometer_dir();
    let purge = args.iter().any(|a| a == "--purge");
    let code = match uninstall(&sp, &claw, &our_command()) {
        Ok(UninstallOutcome::Restored) => {
            println!("uninstalled: original statusLine restored");
            0
        }
        Ok(UninstallOutcome::RemovedKey) => {
            println!("uninstalled: statusLine key removed");
            0
        }
        Ok(UninstallOutcome::NotInstalled) => {
            println!("not installed — nothing to do");
            0
        }
        Ok(UninstallOutcome::NotOurs) => {
            eprintln!(
                "statusLine was changed after install — refusing to touch it.\n\
                 Your original statusLine (if any) is in {}",
                claw.join("wrapped.json").display()
            );
            1
        }
        Err(e) => {
            eprintln!("uninstall aborted: {e}");
            1
        }
    };
    if code == 0 {
        if purge {
            let _ = std::fs::remove_dir_all(&claw);
            println!("purged {}", claw.display());
        } else if claw.exists() {
            println!("left on disk (remove with --purge): {}", claw.display());
        }
    }
    code
}

pub fn cmd_status() -> i32 {
    match read_state(&paths::state_path()) {
        Some(state) => {
            println!("{}", render_statusline(&state));
            println!("captured_at: {}", state.captured_at);
            0
        }
        None => {
            println!("no state yet — run a Claude Code session after `clawdometer install`");
            0
        }
    }
}

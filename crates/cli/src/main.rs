mod commands;
mod hook;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("hook") => {
            let line = std::panic::catch_unwind(hook::run_hook)
                .unwrap_or_else(|_| String::from("clawdometer"));
            use std::io::Write;
            let _ = writeln!(std::io::stdout(), "{line}");
            0
        }
        Some("install") => commands::cmd_install(&args),
        Some("uninstall") => commands::cmd_uninstall(&args),
        Some("status") => commands::cmd_status(),
        _ => {
            eprintln!("usage: clawdometer <hook|install|uninstall|status> [--settings <path>] [--purge]");
            2
        }
    };
    std::process::exit(code);
}

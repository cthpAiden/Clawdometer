mod hook;

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_default();
    match arg.as_str() {
        "hook" => {
            // Safety invariant: NEVER break the user's statusline.
            let line = std::panic::catch_unwind(hook::run_hook)
                .unwrap_or_else(|_| String::from("clawdometer"));
            println!("{line}");
            std::process::exit(0);
        }
        "install" | "uninstall" | "status" => {
            eprintln!("{arg}: not implemented yet");
            std::process::exit(2);
        }
        _ => {
            eprintln!("usage: clawdometer <hook|install|uninstall|status>");
            std::process::exit(2);
        }
    }
}

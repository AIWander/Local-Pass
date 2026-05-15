//! Local-Pass — expose a Windows machine to a remote AI via secure tunnel + bearer token.
//!
//! v0.1.0-alpha: scaffold only. Real HTTP/SSE MCP server, auth subsystem, screen viewer,
//! and tool surface land in subsequent commits.
//!
//! See <https://github.com/AIWander/Local-Pass> for status and install instructions.

fn main() {
    let version = env!("CARGO_PKG_VERSION");
    let args: Vec<String> = std::env::args().collect();

    match args.get(1).map(String::as_str) {
        Some("--version") | Some("-V") => {
            println!("local-pass {version}");
        }
        Some("init") => {
            eprintln!("init subcommand not yet implemented (scaffold v{version}).");
            eprintln!("Will: generate bearer token, write to ./.local-pass/auth.token, print to console.");
            std::process::exit(2);
        }
        Some("serve") => {
            eprintln!("serve subcommand not yet implemented (scaffold v{version}).");
            eprintln!("Will: start HTTP/SSE MCP server on configured bind address.");
            std::process::exit(2);
        }
        Some("rotate-token") => {
            eprintln!("rotate-token subcommand not yet implemented (scaffold v{version}).");
            std::process::exit(2);
        }
        Some("install") | Some("uninstall") => {
            eprintln!("install/uninstall subcommands not yet implemented (scaffold v{version}).");
            std::process::exit(2);
        }
        _ => {
            eprintln!("local-pass v{version} — scaffold, not yet functional.");
            eprintln!("See https://github.com/AIWander/Local-Pass for status.");
        }
    }
}

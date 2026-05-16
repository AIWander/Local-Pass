//! Local-Pass — expose a Windows machine to a remote AI via secure tunnel + bearer token.
//!
//! v0.1.0-alpha: install/uninstall subcommands wired; serve/init/rotate-token are scaffold-only.
//! See https://github.com/AIWander/Local-Pass for status and install instructions.

mod auth;
mod install;

use anyhow::Result;

const SERVER_KEY: &str = "local-pass";
const BINARY_NAME: &str = "local-pass";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let sub = args.get(1).map(|s| s.as_str());

    match sub {
        Some("--version") | Some("-V") => {
            println!("{} {}", BINARY_NAME, env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("--help") | Some("-h") => {
            print_help();
            Ok(())
        }
        Some("install") => install::install(SERVER_KEY, &args[2..]),
        Some("uninstall") => install::uninstall(SERVER_KEY, &args[2..]),
        Some("init") => auth::init(&args[2..]),
        Some("serve") => {
            eprintln!("serve subcommand not yet implemented (scaffold v{}).", env!("CARGO_PKG_VERSION"));
            eprintln!("Will: start HTTP/SSE MCP server on configured bind address (default 127.0.0.1:9100).");
            eprintln!("See https://github.com/AIWander/Local-Pass#manual-operation-until-v1-ships for the manual workaround.");
            std::process::exit(2);
        }
        Some("rotate-token") => auth::rotate(&args[2..]),
        None => {
            eprintln!("local-pass v{} — no subcommand given.", env!("CARGO_PKG_VERSION"));
            eprintln!("Try: local-pass --help");
            std::process::exit(2);
        }
        Some(other) => {
            eprintln!("Unknown subcommand: {}", other);
            print_help();
            std::process::exit(2);
        }
    }
}

fn print_help() {
    println!("Local-Pass v{}", env!("CARGO_PKG_VERSION"));
    println!();
    println!("USAGE:");
    println!("  local-pass init                          Generate bearer token (scaffold-only)");
    println!("  local-pass serve --bind <ip:port>        Start HTTP/SSE MCP server (scaffold-only)");
    println!("  local-pass rotate-token                  Rotate bearer token (scaffold-only)");
    println!("  local-pass install --target <host>       Register with host config as '{}'", SERVER_KEY);
    println!("  local-pass uninstall --target <host>     Unregister from host config");
    println!("  local-pass --version                     Print version");
    println!("  local-pass --help                        Print this help");
    println!();
    install::print_install_help(BINARY_NAME, SERVER_KEY);
    println!();
    println!("Repository: https://github.com/AIWander/Local-Pass");
}

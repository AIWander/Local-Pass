//! Local-Pass — expose a Windows machine to a remote AI via secure tunnel + bearer token.
//!
//! v0.1.3-alpha: install/uninstall/init/rotate-token wired; `serve` runs a real
//! MCP server over rmcp's Streamable HTTP transport (the spec GPT/Claude remote
//! connectors attach to), exposing a curated, profile-filtered, root-scoped tool
//! set. See https://github.com/AIWander/Local-Pass for status and install steps.

mod auth;
mod guard;
mod install;
mod mcp;
mod mcp_tools;
mod profile;
mod psession;
mod server;

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
        Some("serve") => server::run(&args[2..]),
        Some("rotate-token") => auth::rotate(&args[2..]),
        None => {
            eprintln!(
                "local-pass v{} — no subcommand given.",
                env!("CARGO_PKG_VERSION")
            );
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
    println!("  local-pass init                          Generate bearer token");
    println!("  local-pass serve [OPTIONS]               Start the MCP Streamable HTTP server");
    println!("      --bind <ip:port>                     Listen address (default 127.0.0.1:9100)");
    println!("      --profile <lean|full>                Tool exposure profile (default lean)");
    println!("      --root <path>                        Safe filesystem root (default $LOCALPASS_ROOT or home)");
    println!("      --read-only                          Deny write_file + all shell execution");
    println!("  local-pass rotate-token                  Rotate bearer token");
    println!(
        "  local-pass install --target <host>       Register with host config as '{}'",
        SERVER_KEY
    );
    println!("  local-pass uninstall --target <host>     Unregister from host config");
    println!("  local-pass --version                     Print version");
    println!("  local-pass --help                        Print this help");
    println!();
    install::print_install_help(BINARY_NAME, SERVER_KEY);
    println!();
    println!("Repository: https://github.com/AIWander/Local-Pass");
}

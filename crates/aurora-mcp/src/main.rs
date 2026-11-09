use aurora_mcp::{ipc::Discovery, sdk};
use aurora_session::InstallationId;

fn main() {
    if run().is_err() {
        // Never print IO errors: they may contain private discovery paths.
        eprintln!("foldurora-mcp: invalid configuration or local transport unavailable");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--help"] {
        println!(
            "foldurora-mcp --installation <viewer-installation-id>\nStdio host MCP; host and profile authorization are required separately."
        );
        return Ok(());
    }
    if args == ["--version"] {
        println!("foldurora-mcp {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.len() != 2 || args[0] != "--installation" {
        return Err("expected explicit viewer installation".into());
    }
    let installation = InstallationId::parse_hex(&args[1])?;
    let discovery = Discovery::production(installation)?;
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(sdk::serve(discovery, tokio::io::stdin(), tokio::io::stdout()))?;
    Ok(())
}

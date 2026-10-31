use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;
use taur_core::{Store, resolve_home};

/// TAUR: an AUR-compatible package server.
#[derive(Parser)]
#[command(name = "taur", version)]
struct Cli {
    /// Data directory (default: $TAUR_HOME, else ~/.local/share/taur).
    #[arg(long, global = true)]
    home: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the HTTP server (AUR RPC, git, snapshots, dumps, account API).
    Serve {
        #[arg(long, default_value = "127.0.0.1:8080")]
        bind: String,
    },
    /// Import a package directory containing .SRCINFO (and PKGBUILD).
    Import {
        dir: PathBuf,
        /// Existing user to set as maintainer (orphan if omitted).
        #[arg(long)]
        maintainer: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let store = Store::open(&resolve_home(cli.home))?;
    match cli.cmd {
        Cmd::Serve { bind } => {
            let listener = tokio::net::TcpListener::bind(&bind).await?;
            eprintln!("taur: listening on http://{}", listener.local_addr()?);
            axum::serve(listener, taur_server::app(Arc::new(store))).await?;
        }
        Cmd::Import { dir, maintainer } => {
            let info = store.import_dir(&dir, maintainer.as_deref())?;
            println!("imported {} {}", info.pkgbase, info.version());
        }
    }
    Ok(())
}

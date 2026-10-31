//! `taur` command-line definition (server and admin tasks).

use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// TAUR: an AUR-compatible package server.
#[derive(Parser, Debug)]
#[command(name = "taur", version)]
pub struct Cli {
    /// Data directory (default: $TAUR_HOME, else ~/.local/share/taur).
    #[arg(long, global = true)]
    pub home: Option<PathBuf>,
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Run the HTTP server (AUR RPC, git, snapshots, dumps, API) and optional SSH server.
    Serve {
        #[arg(long, default_value = "127.0.0.1:8080")]
        bind: String,
        /// Also serve git over SSH on this address (e.g. 0.0.0.0:2222).
        #[arg(long)]
        ssh_bind: Option<String>,
    },
    /// Import a package directory containing .SRCINFO (and PKGBUILD).
    Import {
        dir: PathBuf,
        /// Existing user to set as maintainer (orphan if omitted).
        #[arg(long)]
        maintainer: Option<String>,
    },
    /// Git hook entry point (run by git during pushes; not for manual use).
    #[command(hide = true)]
    Hook { name: String },
}

/// Render man pages for `taur` and its visible subcommands: `(file name, roff)`.
pub fn man_pages() -> Vec<(String, Vec<u8>)> {
    let mut cmd = <Cli as clap::CommandFactory>::command();
    cmd.build();
    let mut out = Vec::new();
    let mut buf = Vec::new();
    let _ = clap_mangen::Man::new(cmd.clone()).render(&mut buf);
    out.push(("taur.1".to_string(), buf));
    for sub in cmd
        .get_subcommands()
        .filter(|s| !s.is_hide_set() && s.get_name() != "help")
    {
        let name: &'static str = format!("taur-{}", sub.get_name()).leak();
        let mut buf = Vec::new();
        let _ = clap_mangen::Man::new(sub.clone().name(name)).render(&mut buf);
        out.push((format!("{name}.1"), buf));
    }
    out
}

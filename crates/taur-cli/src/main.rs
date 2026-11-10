use clap::Parser;

fn main() {
    // ureq talks to the server with rustls. The same crate is also built with
    // aws-lc-rs elsewhere in the workspace, so rustls will not choose a
    // provider on its own.
    let _ = rustls::crypto::ring::default_provider().install_default();
    std::process::exit(taur_cli::run(taur_cli::Cli::parse()));
}

use clap::Parser;

fn main() {
    std::process::exit(taur_cli::run(taur_cli::Cli::parse()));
}

use anyhow::Result;
use clap::Parser;

mod canvas;
mod capture;
mod cli;
mod config;
mod client;
mod daemon;
mod dbus;
mod error;
mod export;
mod ui;

fn main() -> Result<()> {
    init_tracing();
    let cli = cli::Cli::parse();
    match cli.command {
        None => daemon::run(),
        Some(cmd) => client::run(cmd),
    }
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

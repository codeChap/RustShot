use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "rustshot",
    version,
    about = "Fast Rust screenshot tool for Linux + X11 (Flameshot-compatible)"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Interactive region-select GUI on the cursor's screen.
    Gui(CaptureArgs),
    /// Capture all monitors immediately, no UI.
    Full(CaptureArgs),
    /// Capture a single screen immediately, no UI.
    Screen {
        #[command(flatten)]
        common: CaptureArgs,
        /// Screen index (0-based). If omitted, uses the cursor's screen.
        #[arg(short = 'n', long)]
        number: Option<usize>,
    },
}

#[derive(Args, Debug, Clone)]
pub struct CaptureArgs {
    /// Save screenshot to PATH (PNG).
    #[arg(short = 'p', long)]
    pub path: Option<PathBuf>,
    /// Copy screenshot to clipboard.
    #[arg(short = 'c', long)]
    pub clipboard: bool,
    /// Delay before capture (ms).
    #[arg(short = 'd', long, default_value_t = 0)]
    pub delay: u64,
    /// Don't save to disk (useful with --clipboard for clipboard-only capture).
    #[arg(long)]
    pub no_save: bool,
}

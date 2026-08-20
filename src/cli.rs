use clap::{ArgGroup, Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "cc-switch-cli",
    version,
    about = "Switch CC Switch providers from the command line"
)]
pub struct Cli {
    /// Emit machine-readable JSON.
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Inspect or switch Codex providers.
    Codex(CodexArgs),
}

#[derive(Debug, Args)]
#[command(group(
    ArgGroup::new("action")
        .required(true)
        .multiple(false)
        .args(["name", "id", "list", "current"])
))]
pub struct CodexArgs {
    /// Switch to the provider whose display name matches exactly.
    #[arg(long, value_name = "NAME")]
    pub name: Option<String>,

    /// Switch to the provider with this exact UUID.
    #[arg(long, value_name = "UUID")]
    pub id: Option<String>,

    /// List configured Codex providers.
    #[arg(long)]
    pub list: bool,

    /// Show the current Codex provider.
    #[arg(long)]
    pub current: bool,
}

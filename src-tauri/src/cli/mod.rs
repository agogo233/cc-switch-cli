use clap::{Parser, Subcommand};
use clap_complete::Shell;

pub mod commands;
pub mod editor;
pub mod i18n;
pub mod interactive;
pub mod terminal;
pub mod tui;
pub mod ui;

use crate::app_config::AppType;

#[derive(Parser)]
#[command(
    name = "cc-switch",
    version,
    about = "All-in-One Assistant for Claude Code, Codex & Gemini CLI",
    long_about = "Unified management for Claude Code, Codex & Gemini CLI provider configurations, MCP servers, Skills extensions, and system prompts.\n\nRun without arguments to enter interactive mode."
)]
pub struct Cli {
    /// Specify the application type
    #[arg(short, long, global = true, value_enum)]
    pub app: Option<AppType>,

    /// Enable verbose output
    #[arg(short, long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Manage providers (list, add, edit, delete, switch)
    #[command(subcommand)]
    Provider(commands::provider::ProviderCommand),

    /// Manage MCP servers (list, add, edit, delete, sync)
    #[command(subcommand)]
    Mcp(commands::mcp::McpCommand),

    /// Manage prompts (list, activate, edit)
    #[command(subcommand)]
    Prompts(commands::prompts::PromptsCommand),

    /// Manage skills (list, install, uninstall)
    #[command(subcommand)]
    Skills(commands::skills::SkillsCommand),

    /// Manage configuration (export, import, backup, restore)
    #[command(subcommand)]
    Config(commands::config::ConfigCommand),

    /// Manage local multi-app proxy
    #[command(subcommand)]
    Proxy(commands::proxy::ProxyCommand),

    /// Manage environment variables
    #[command(subcommand)]
    Env(commands::env::EnvCommand),

    /// Update cc-switch binary to latest release
    Update(commands::update::UpdateCommand),

    /// Enter interactive mode
    #[command(alias = "ui")]
    Interactive,

    /// Generate shell completions
    Completions {
        /// The shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },
}

/// Generate shell completions
pub fn generate_completions(shell: Shell) {
    use clap::CommandFactory;
    let mut cmd = Cli::command();
    let name = cmd.get_name().to_string();
    clap_complete::generate(shell, &mut cmd, name, &mut std::io::stdout());
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{Cli, Commands};

    #[test]
    fn parses_proxy_serve_subcommand() {
        let cli = Cli::parse_from(["cc-switch", "proxy", "serve", "--listen-port", "0"]);

        match cli.command {
            Some(Commands::Proxy(super::commands::proxy::ProxyCommand::Serve {
                listen_port,
                ..
            })) => {
                assert_eq!(listen_port, Some(0));
            }
            _ => panic!("expected proxy serve command"),
        }
    }

    #[test]
    fn parses_proxy_serve_takeover_flags() {
        let cli = Cli::parse_from([
            "cc-switch",
            "proxy",
            "serve",
            "--takeover",
            "claude",
            "--takeover",
            "codex",
        ]);

        match cli.command {
            Some(Commands::Proxy(super::commands::proxy::ProxyCommand::Serve {
                takeovers,
                ..
            })) => {
                assert_eq!(
                    takeovers,
                    vec![super::AppType::Claude, super::AppType::Codex]
                );
            }
            _ => panic!("expected proxy serve command with takeover flags"),
        }
    }

    #[test]
    fn parses_proxy_enable_subcommand() {
        let cli = Cli::parse_from(["cc-switch", "proxy", "enable"]);

        match cli.command {
            Some(Commands::Proxy(super::commands::proxy::ProxyCommand::Enable)) => {}
            _ => panic!("expected proxy enable command"),
        }
    }

    #[test]
    fn parses_proxy_disable_subcommand() {
        let cli = Cli::parse_from(["cc-switch", "proxy", "disable"]);

        match cli.command {
            Some(Commands::Proxy(super::commands::proxy::ProxyCommand::Disable)) => {}
            _ => panic!("expected proxy disable command"),
        }
    }
}

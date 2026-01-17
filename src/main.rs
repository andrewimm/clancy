mod config;
mod project;
mod repl;
mod transcript;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "clancy")]
#[command(about = "Claude Code session harness with cross-session memory")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start a session — enters the Clancy REPL
    Start {
        /// Project name
        project_name: String,
    },
    /// List all projects
    List,
    /// Show project status and notes
    Status {
        /// Project name (optional, defaults to current)
        project_name: Option<String>,
    },
    /// View/edit notes directly
    Notes {
        /// Project name
        project: String,
        /// Note category (architecture, decisions, failures, plan)
        category: Option<String>,
    },
    /// Archive a completed project
    Archive {
        /// Project name
        project_name: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Start { project_name } => {
            repl::start_session(&project_name)?;
        }
        Commands::List => {
            project::list_projects()?;
        }
        Commands::Status { project_name } => {
            project::show_status(project_name.as_deref())?;
        }
        Commands::Notes { project, category } => {
            project::edit_notes(&project, category.as_deref())?;
        }
        Commands::Archive { project_name } => {
            project::archive_project(&project_name)?;
        }
    }

    Ok(())
}

use std::{io::stdin, process::ExitCode};

use aptly_rest::AptlyRest;
use clap::{Parser, Subcommand, ValueEnum};
use color_eyre::{eyre::ensure, Result};
use mirror::MirrorCommand;
use publish::PublishCommand;
use repo::RepoCommand;
use snapshot::SnapshotCommand;
use tools::ToolsCommand;
use tracing::{info, metadata::LevelFilter};
use tracing_error::ErrorLayer;
use tracing_subscriber::prelude::*;

mod mirror;
mod publish;
mod repo;
mod snapshot;
mod tools;

#[derive(ValueEnum, Clone, Copy, Debug, Default)]
enum OutputFormat {
    #[default]
    Name,
    Json,
    Yaml,
}

/// Return the given names, or read them from stdin, one per line, if the
/// only name given is `-`
fn names_or_stdin(names: Vec<String>) -> Result<Vec<String>> {
    if names != ["-"] {
        return Ok(names);
    }

    let mut names = vec![];
    for line in stdin().lines() {
        let line = line?;
        let name = line.trim();
        if !name.is_empty() {
            names.push(name.to_owned());
        }
    }
    ensure!(!names.is_empty(), "No names given on stdin");
    Ok(names)
}

#[derive(Subcommand, Debug)]
enum Command {
    Repo {
        #[clap(subcommand)]
        command: RepoCommand,
    },
    Publish {
        #[clap(subcommand)]
        command: PublishCommand,
    },
    Snapshot {
        #[clap(subcommand)]
        command: SnapshotCommand,
    },
    Tools {
        #[clap(subcommand)]
        command: ToolsCommand,
    },
    Mirror {
        #[clap(subcommand)]
        command: MirrorCommand,
    },
    DbCleanup,
}

#[derive(Parser, Debug)]
struct Opts {
    #[clap(subcommand)]
    command: Command,
    /// Url for the aptly rest API endpoint
    #[clap(
        short = 'u',
        long,
        env = "APTLY_API_URL",
        default_value = "http://localhost:8080"
    )]
    api_url: url::Url,
    /// Authentication token for the API
    #[clap(long, env = "APTLY_API_TOKEN")]
    api_token: Option<String>,
}

#[tokio::main]
async fn main() -> Result<ExitCode> {
    tracing_subscriber::registry()
        .with(ErrorLayer::default())
        .with(tracing_subscriber::fmt::layer().with_filter(LevelFilter::INFO))
        .init();
    color_eyre::install().unwrap();
    let opts = Opts::parse();
    let aptly = if let Some(token) = opts.api_token {
        AptlyRest::new_with_token(opts.api_url, &token)?
    } else {
        AptlyRest::new(opts.api_url)
    };

    match opts.command {
        Command::Repo { command } => command.run(&aptly).await,
        Command::Publish { command } => command.run(&aptly).await,
        Command::Snapshot { command } => command.run(&aptly).await,
        Command::Tools { command } => command.run().await,
        Command::Mirror { command } => command.run(&aptly).await,
        Command::DbCleanup => {
            aptly.db_cleanup().await?;
            info!("Ran database cleanup");
            Ok(ExitCode::SUCCESS)
        }
    }
}

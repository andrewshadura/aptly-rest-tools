use std::{io::stdout, process::ExitCode};

use aptly_rest::{api::snapshots, AptlyRest, AptlyRestError};
use clap::{Parser, Subcommand};
use color_eyre::Result;
use http::StatusCode;
use tracing::info;

use crate::{names_or_stdin, OutputFormat};

#[derive(Parser, Debug)]
pub struct SnapshotListOpts {
    #[clap(long, value_enum, default_value_t)]
    format: OutputFormat,
}

#[derive(Parser, Debug)]
pub struct SnapshotTestExistsOpts {
    snapshot: String,
}

#[derive(Parser, Debug)]
pub struct SnapshotDropOpts {
    /// Snapshots to drop; several are dropped at once, all or none; `-` reads
    /// them from stdin, one per line
    #[clap(required = true)]
    snapshots: Vec<String>,
    /// Drop snapshots even if other snapshots use them as a source
    #[clap(long)]
    force: bool,
}

#[derive(Subcommand, Debug)]
pub enum SnapshotCommand {
    List(SnapshotListOpts),
    TestExists(SnapshotTestExistsOpts),
    Drop(SnapshotDropOpts),
}

impl SnapshotCommand {
    pub async fn run(self, aptly: &AptlyRest) -> Result<ExitCode> {
        match self {
            SnapshotCommand::List(args) => {
                let snapshots = aptly.snapshots().await?;
                match args.format {
                    OutputFormat::Name => {
                        let mut names: Vec<_> = snapshots.iter().map(|s| s.name()).collect();
                        names.sort();
                        for name in names {
                            println!("{name}");
                        }
                    }
                    OutputFormat::Json => {
                        serde_json::to_writer_pretty(&mut stdout(), &snapshots)?;
                        println!();
                    }
                    OutputFormat::Yaml => {
                        serde_saphyr::to_io_writer(&mut stdout(), &snapshots)?;
                        println!();
                    }
                }
            }

            SnapshotCommand::TestExists(args) => {
                if let Err(err) = aptly.snapshot(&args.snapshot).get().await {
                    if let AptlyRestError::Request(err) = &err {
                        if err.status() == Some(StatusCode::NOT_FOUND) {
                            return Ok(ExitCode::FAILURE);
                        }
                    }

                    return Err(err.into());
                }
            }

            SnapshotCommand::Drop(args) => {
                let names = names_or_stdin(args.snapshots)?;
                let options = snapshots::DeleteOptions { force: args.force };
                if let [snapshot] = names.as_slice() {
                    aptly.snapshot(snapshot).delete(&options).await?;
                } else {
                    aptly.delete_snapshots(&names, &options).await?;
                }
                for snapshot in &names {
                    info!("Deleted snapshot '{snapshot}'");
                }
            }
        }

        Ok(ExitCode::SUCCESS)
    }
}

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod cli;
mod db;
mod generator;
mod handlers;
mod metrics;
mod serve;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "aws_mock_data_service=info,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Cli::parse();
    let pool = db::init(&cli.database_url).await?;

    match cli.command {
        Commands::Gen {
            endpoint_url,
            region,
            scenario,
            prune,
            json,
        } => {
            generator::run(pool, endpoint_url, region, scenario, prune, json).await?;
        }
        Commands::Serve { port, address } => {
            serve::run(pool, address, port).await?;
        }
    }

    Ok(())
}

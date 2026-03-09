use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, Deserialize, PartialEq)]
pub enum Scenario {
    Baseline,
    Spike,
    IdleHeavy,
}

impl std::fmt::Display for Scenario {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Scenario::Baseline => write!(f, "Baseline"),
            Scenario::Spike => write!(f, "Spike"),
            Scenario::IdleHeavy => write!(f, "Idle-Heavy"),
        }
    }
}

#[derive(Parser)]
#[command(name = "aws-mock", about = "AWS Mock Data Service", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Database URL (sqlite:path/to/db.sqlite)
    #[arg(
        short,
        long,
        env = "DATABASE_URL",
        default_value = "sqlite:mock_data.db"
    )]
    pub database_url: String,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Generate mock data by discovering resources in LocalStack
    Gen {
        /// LocalStack endpoint URL
        #[arg(
            long,
            env = "AWS_ENDPOINT_URL",
            default_value = "http://localhost:4566"
        )]
        endpoint_url: String,

        /// AWS region
        #[arg(short, long, env = "AWS_DEFAULT_REGION", default_value = "us-east-1")]
        region: String,

        /// Scenario to apply
        #[arg(short, long, value_enum, default_value_t = Scenario::Baseline)]
        scenario: Scenario,

        /// Prune resources that are no longer in LocalStack
        #[arg(long)]
        prune: bool,

        /// Output JSON summary of discovered resources
        #[arg(long)]
        json: bool,
    },
    /// Start the AWS-compatible API server
    Serve {
        /// Port to listen on
        #[arg(short, long, default_value_t = 8080)]
        port: u16,

        /// Address to bind to
        #[arg(short, long, default_value = "127.0.0.1")]
        address: String,
    },
}

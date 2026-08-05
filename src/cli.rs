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
#[command(name = "foxtail", about = "Foxtail native commands", version)]
pub struct NativeCli {
    #[command(subcommand)]
    pub command: Commands,
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
    /// Publish and inspect the versioned release-qualification fixture.
    #[command(name = "fixture")]
    Fixture {
        #[command(subcommand)]
        command: FixtureCommands,
    },
}

#[derive(Subcommand)]
pub enum FixtureCommands {
    /// Print the canonical release-qualification Fixture Definition.
    Definition {
        /// Fixture version.
        #[arg(long, default_value = "release-qualification-v1")]
        version: String,
    },
    /// Realize one fixture against the discovered EC2 estate.
    Realize {
        /// Fixture version.
        #[arg(long, default_value = "release-qualification-v1")]
        version: String,
        /// RFC3339 UTC anchor for deterministic evidence windows.
        #[arg(long)]
        clock_anchor: Option<String>,
        /// AWS account scope to publish in the manifest.
        #[arg(long)]
        account_id: Option<String>,
        /// AWS region; must match the discovered EC2 estate.
        #[arg(long)]
        region: Option<String>,
        /// LocalStack endpoint provenance to publish in the manifest.
        #[arg(long)]
        endpoint_url: Option<String>,
        /// LocalStack version provenance to publish in the manifest.
        #[arg(long)]
        localstack_version: Option<String>,
    },
    /// Print the persisted fixture status and digests.
    Status,
    /// Print the exact persisted canonical Fixture Manifest.
    Manifest,
    /// Print realized control identities and their manifest digest.
    Identities,
}

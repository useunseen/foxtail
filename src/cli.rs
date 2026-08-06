use clap::{Args, Parser, Subcommand, ValueEnum};
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
        /// AWS account scope; must match the public mock account identity.
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
    /// Print qualification-only mutation generation state.
    MutationStatus,
    /// Apply one one-use, manifest-bound EC2 fault.
    Fault {
        #[command(flatten)]
        authority: FixtureMutationAuthorityArgs,
        #[arg(long)]
        control_id: String,
        #[arg(long)]
        target_id: String,
        #[arg(long, default_value = "target")]
        scope: String,
        #[arg(long)]
        fault_kind: String,
        #[arg(long)]
        application_time: Option<String>,
    },
    /// Reset one fault using its one-use reset token.
    Reset {
        #[command(flatten)]
        authority: FixtureMutationAuthorityArgs,
        #[arg(long)]
        receipt_id: String,
        #[arg(long)]
        reset_token: String,
    },
    /// Atomically retire the current mutation generation and create a fresh one.
    Recreate {
        #[command(flatten)]
        authority: FixtureMutationAuthorityArgs,
        #[arg(long)]
        clock_anchor: Option<String>,
    },
    /// Destroy the current mutation generation and prove public identity absence.
    Destroy {
        #[command(flatten)]
        authority: FixtureMutationAuthorityArgs,
    },
}

#[derive(Debug, Clone, Args)]
pub struct FixtureMutationAuthorityArgs {
    #[arg(long, default_value = "release-qualification-v1")]
    pub version: String,
    #[arg(long)]
    pub generation: i64,
    #[arg(long)]
    pub manifest_digest: String,
    #[arg(long)]
    pub mutation_generation: i64,
    #[arg(long)]
    pub mutation_generation_id: String,
}

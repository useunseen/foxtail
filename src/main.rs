use anyhow::Result;
use clap::Parser;
use foxtail::cli::{Commands, FixtureCommands, FixtureMutationAuthorityArgs, NativeCli};
use foxtail::wrapper::{
    RunMode, build_invocation, help_text, parse_cli_args, render_debug_line, version_text,
};
use std::env;
use std::ffi::OsString;
use std::process::{self, Stdio};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "foxtail=info,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let raw_args = env::args_os().skip(1).collect::<Vec<_>>();
    let parsed = parse_cli_args(raw_args).map_err(anyhow::Error::msg)?;

    match parsed.mode {
        RunMode::Help => {
            print!("{}", help_text());
            return Ok(());
        }
        RunMode::Version => {
            println!("{}", version_text());
            return Ok(());
        }
        RunMode::Execute => {}
    }

    if is_native_command(&parsed.forwarded_args) {
        let native_args = std::iter::once(OsString::from("foxtail"))
            .chain(parsed.forwarded_args.iter().cloned())
            .collect::<Vec<_>>();
        let native_cli = NativeCli::parse_from(native_args);
        run_native(native_cli.command, &parsed.config.database_url).await?;
        return Ok(());
    }

    let invocation = build_invocation(&parsed);

    if parsed.config.debug_routing {
        eprintln!("{}", render_debug_line(&invocation));
    }

    let status = invocation
        .command()
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;

    process::exit(status.code().unwrap_or(1));
}

fn is_native_command(args: &[OsString]) -> bool {
    matches!(
        args.first().and_then(|arg| arg.to_str()),
        Some("gen" | "serve" | "fixture")
    )
}

async fn run_native(command: Commands, database_url: &str) -> Result<()> {
    let pool = foxtail::db::init(database_url).await?;

    match command {
        Commands::Gen {
            endpoint_url,
            region,
            scenario,
            prune,
            json,
        } => {
            foxtail::generator::run(pool, endpoint_url, region, scenario, prune, json).await?;
        }
        Commands::Serve { port, address } => {
            foxtail::serve::run(pool, address, port).await?;
        }
        Commands::Fixture { command } => {
            run_fixture_command(pool, command).await?;
        }
    }

    Ok(())
}

async fn run_fixture_command(pool: sqlx::SqlitePool, command: FixtureCommands) -> Result<()> {
    let bytes = match command {
        FixtureCommands::Definition { version } => {
            foxtail::fixture::validate_version(Some(&version))?;
            foxtail::fixture::canonical_definition()?.0
        }
        FixtureCommands::Realize {
            version,
            clock_anchor,
            account_id,
            region,
            endpoint_url,
            localstack_version,
        } => foxtail::fixture::realization_response(
            &foxtail::fixture::realize(
                &pool,
                foxtail::fixture::RealizeRequest {
                    version: Some(version),
                    clock_anchor,
                    account_id,
                    region,
                    endpoint_url,
                    localstack_version,
                },
            )
            .await?,
        )?,
        FixtureCommands::Status => foxtail::fixture::read_state(&pool).await?.status_bytes,
        FixtureCommands::Manifest => foxtail::fixture::read_state(&pool)
            .await?
            .manifest_bytes
            .ok_or_else(|| anyhow::anyhow!("fixture has not been realized"))?,
        FixtureCommands::Identities => foxtail::fixture::read_state(&pool).await?.identities_bytes,
        FixtureCommands::MutationStatus => foxtail::fixture::mutation_status(&pool).await?,
        FixtureCommands::Fault {
            authority,
            control_id,
            target_id,
            scope,
            fault_kind,
            application_time,
        } => {
            foxtail::fixture::apply_fault(
                &pool,
                foxtail::fixture::FaultRequest {
                    authority: mutation_authority(authority),
                    control_id,
                    target_id,
                    scope,
                    fault_kind,
                    application_time,
                },
            )
            .await?
        }
        FixtureCommands::Reset {
            authority,
            receipt_id,
            reset_token,
        } => {
            foxtail::fixture::reset_fault(
                &pool,
                foxtail::fixture::ResetRequest {
                    authority: mutation_authority(authority),
                    receipt_id,
                    reset_token,
                },
            )
            .await?
        }
        FixtureCommands::Recreate {
            authority,
            clock_anchor,
        } => {
            foxtail::fixture::recreate(
                &pool,
                foxtail::fixture::RecreateRequest {
                    authority: mutation_authority(authority),
                    clock_anchor,
                },
            )
            .await?
        }
        FixtureCommands::Destroy { authority } => {
            foxtail::fixture::destroy(
                &pool,
                foxtail::fixture::DestroyRequest {
                    authority: mutation_authority(authority),
                },
            )
            .await?
        }
    };
    print!("{}", foxtail::fixture::cli_bytes_to_string(&bytes)?);
    Ok(())
}

fn mutation_authority(args: FixtureMutationAuthorityArgs) -> foxtail::fixture::MutationAuthority {
    foxtail::fixture::MutationAuthority {
        version: Some(args.version),
        generation: Some(args.generation),
        manifest_digest: Some(args.manifest_digest),
        mutation_generation: Some(args.mutation_generation),
        mutation_generation_id: Some(args.mutation_generation_id),
    }
}

use anyhow::Result;
use clap::Parser;
use foxtail::cli::{Commands, NativeCli};
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
        Some("gen" | "serve")
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
    }

    Ok(())
}

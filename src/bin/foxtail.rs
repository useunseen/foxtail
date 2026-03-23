use anyhow::Result;
use aws_mock_data_service::wrapper::{
    RunMode, build_invocation, help_text, parse_cli_args, render_debug_line, version_text,
};
use std::env;
use std::process::{self, Stdio};

fn main() -> Result<()> {
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

use std::env;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::process::Command;

const DEFAULT_FOXTAIL_ENDPOINT: &str = "http://127.0.0.1:8080";
const DEFAULT_DATABASE_URL: &str = "sqlite:mock_data.db";
const DEFAULT_AWS_BIN: &str = "aws";
const DEFAULT_AWSLOCAL_BIN: &str = "awslocal";
const VERSION: &str = env!("CARGO_PKG_VERSION");

const GLOBAL_FLAGS_WITH_VALUES: &[&str] = &[
    "--ca-bundle",
    "--cli-binary-format",
    "--cli-connect-timeout",
    "--cli-read-timeout",
    "--color",
    "--endpoint-url",
    "--output",
    "--profile",
    "--query",
    "--region",
];

const FOXTAIL_ROUTED_COMMANDS: &[(&str, &str)] = &[
    ("ce", "get-cost-and-usage"),
    ("ce", "get-cost-and-usage-with-resources"),
    ("ce", "get-cost-forecast"),
    ("ce", "get-usage-forecast"),
    ("ce", "get-dimension-values"),
    ("ce", "get-tags"),
    ("ce", "get-reservation-coverage"),
    ("ce", "get-reservation-utilization"),
    ("ce", "get-savings-plans-coverage"),
    ("ce", "get-savings-plans-utilization"),
    ("ce", "get-rightsizing-recommendation"),
    ("ce", "get-anomalies"),
    ("ce", "get-anomaly-monitors"),
    ("ce", "get-anomaly-subscriptions"),
    ("resourcegroupstaggingapi", "get-resources"),
    ("resourcegroupstaggingapi", "get-tag-keys"),
    ("resourcegroupstaggingapi", "get-tag-values"),
    ("pricing", "get-products"),
    ("compute-optimizer", "get-ec2-instance-recommendations"),
    ("compute-optimizer", "get-ebs-volume-recommendations"),
    ("cur", "describe-report-definitions"),
    ("cloudwatch", "list-metrics"),
    ("cloudwatch", "get-metric-statistics"),
    ("cloudwatch", "get-metric-data"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Aws,
    Awslocal,
}

impl fmt::Display for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Backend::Aws => write!(f, "foxtail"),
            Backend::Awslocal => write!(f, "awslocal"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub database_url: String,
    pub foxtail_endpoint: String,
    pub aws_bin: String,
    pub awslocal_bin: String,
    pub debug_routing: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            database_url: env::var("DATABASE_URL")
                .unwrap_or_else(|_| DEFAULT_DATABASE_URL.to_string()),
            foxtail_endpoint: env::var("FOXTAIL_ENDPOINT_URL")
                .unwrap_or_else(|_| DEFAULT_FOXTAIL_ENDPOINT.to_string()),
            aws_bin: env::var("FOXTAIL_AWS_BIN").unwrap_or_else(|_| DEFAULT_AWS_BIN.to_string()),
            awslocal_bin: env::var("FOXTAIL_AWSLOCAL_BIN")
                .unwrap_or_else(|_| DEFAULT_AWSLOCAL_BIN.to_string()),
            debug_routing: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCli {
    pub config: Config,
    pub forwarded_args: Vec<OsString>,
    pub mode: RunMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunMode {
    Execute,
    Help,
    Version,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub backend: Backend,
    pub program: String,
    pub args: Vec<OsString>,
}

impl Invocation {
    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.args);
        command
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandTarget {
    pub service: String,
    pub operation: String,
}

pub fn parse_cli_args<I>(args: I) -> Result<ParsedCli, String>
where
    I: IntoIterator<Item = OsString>,
{
    let mut config = Config::default();
    let mut forwarded_args = Vec::new();
    let mut mode = RunMode::Execute;
    let mut passthrough = false;
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        if passthrough {
            forwarded_args.push(arg);
            continue;
        }

        match arg.to_str() {
            Some("--") => {
                passthrough = true;
            }
            Some("--debug-routing") => {
                config.debug_routing = true;
            }
            Some("--database-url") | Some("-d") => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--database-url requires a value".to_string())?;
                config.database_url = value.to_string_lossy().into_owned();
            }
            Some(flag) if flag.starts_with("--database-url=") => {
                config.database_url = flag.trim_start_matches("--database-url=").to_string();
            }
            Some("--foxtail-endpoint") => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--foxtail-endpoint requires a value".to_string())?;
                config.foxtail_endpoint = value.to_string_lossy().into_owned();
            }
            Some(flag) if flag.starts_with("--foxtail-endpoint=") => {
                config.foxtail_endpoint =
                    flag.trim_start_matches("--foxtail-endpoint=").to_string();
            }
            Some("--awslocal-bin") => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--awslocal-bin requires a value".to_string())?;
                config.awslocal_bin = value.to_string_lossy().into_owned();
            }
            Some(flag) if flag.starts_with("--awslocal-bin=") => {
                config.awslocal_bin = flag.trim_start_matches("--awslocal-bin=").to_string();
            }
            Some("--aws-bin") => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--aws-bin requires a value".to_string())?;
                config.aws_bin = value.to_string_lossy().into_owned();
            }
            Some(flag) if flag.starts_with("--aws-bin=") => {
                config.aws_bin = flag.trim_start_matches("--aws-bin=").to_string();
            }
            Some("--help") | Some("-h") => {
                mode = RunMode::Help;
            }
            Some("--version") | Some("-V") => {
                mode = RunMode::Version;
            }
            _ => {
                forwarded_args.push(arg);
                forwarded_args.extend(iter);
                break;
            }
        }
    }

    if forwarded_args.is_empty() && mode == RunMode::Execute {
        mode = RunMode::Help;
    }

    Ok(ParsedCli {
        config,
        forwarded_args,
        mode,
    })
}

pub fn classify_target(args: &[OsString]) -> Option<CommandTarget> {
    let mut index = 0;
    let mut service = None;
    let mut operation = None;

    while index < args.len() {
        let arg = args[index].to_string_lossy();

        if arg.starts_with("--") {
            if let Some(flag) = arg.split('=').next()
                && GLOBAL_FLAGS_WITH_VALUES.contains(&flag)
                && !arg.contains('=')
            {
                index += 2;
                continue;
            }

            index += 1;
            continue;
        }

        if service.is_none() {
            service = Some(arg.to_ascii_lowercase());
            index += 1;
            continue;
        }

        if operation.is_none() {
            operation = Some(arg.to_ascii_lowercase());
            break;
        }
    }

    match (service, operation) {
        (Some(service), Some(operation)) => Some(CommandTarget { service, operation }),
        _ => None,
    }
}

pub fn route_backend(target: Option<&CommandTarget>) -> Backend {
    match target {
        Some(target)
            if FOXTAIL_ROUTED_COMMANDS
                .contains(&(target.service.as_str(), target.operation.as_str())) =>
        {
            Backend::Aws
        }
        _ => Backend::Awslocal,
    }
}

pub fn has_explicit_endpoint(args: &[OsString]) -> bool {
    args.iter().any(|arg| {
        let value = arg.to_string_lossy();
        value == "--endpoint-url" || value.starts_with("--endpoint-url=")
    })
}

pub fn build_invocation(parsed: &ParsedCli) -> Invocation {
    let target = classify_target(&parsed.forwarded_args);
    let backend = route_backend(target.as_ref());

    match backend {
        Backend::Aws => {
            let mut args = Vec::new();
            if !has_explicit_endpoint(&parsed.forwarded_args) {
                args.push(OsString::from("--endpoint-url"));
                args.push(OsString::from(&parsed.config.foxtail_endpoint));
            }
            args.extend(parsed.forwarded_args.iter().cloned());

            Invocation {
                backend,
                program: parsed.config.aws_bin.clone(),
                args,
            }
        }
        Backend::Awslocal => Invocation {
            backend,
            program: parsed.config.awslocal_bin.clone(),
            args: parsed.forwarded_args.clone(),
        },
    }
}

pub fn help_text() -> String {
    format!(
        "\
foxtail {VERSION}

Usage:
  foxtail [foxtail options] <command>
  foxtail [foxtail options] <service> <operation> [aws args...]

Purpose:
  Single local entrypoint for Foxtail data generation, service hosting, and
  mixed LocalStack + Foxtail AWS CLI workflows.

Native commands:
  gen                           Discover LocalStack resources and seed mock data
  serve                         Start the AWS-compatible Foxtail API service

Routing rules:
  1. Native commands such as `gen` and `serve` run in this process.
  2. If `(service, operation)` is in the Foxtail support table, foxtail runs:
       aws --endpoint-url http://127.0.0.1:8080 ...
  3. Otherwise foxtail runs:
       awslocal ...
  4. If you already pass `--endpoint-url`, foxtail preserves your explicit value.

Foxtail options:
  -d, --database-url <url>      Database URL for native gen/serve commands
  --debug-routing              Print the selected backend and effective command to stderr
  --foxtail-endpoint <url>     Override the Foxtail endpoint for routed commands
  --aws-bin <path>             Override the aws executable for routed commands
  --awslocal-bin <path>        Override the awslocal executable for passthrough commands
  -h, --help                   Show this help text
  -V, --version                Show the wrapper version

Foxtail-routed commands:
  ce get-cost-and-usage
  ce get-cost-and-usage-with-resources
  ce get-cost-forecast
  ce get-usage-forecast
  ce get-dimension-values
  ce get-tags
  ce get-reservation-coverage
  ce get-reservation-utilization
  ce get-savings-plans-coverage
  ce get-savings-plans-utilization
  ce get-rightsizing-recommendation
  ce get-anomalies
  ce get-anomaly-monitors
  ce get-anomaly-subscriptions
  resourcegroupstaggingapi get-resources
  resourcegroupstaggingapi get-tag-keys
  resourcegroupstaggingapi get-tag-values
  pricing get-products
  compute-optimizer get-ec2-instance-recommendations
  compute-optimizer get-ebs-volume-recommendations
  cur describe-report-definitions
  cloudwatch list-metrics
  cloudwatch get-metric-statistics
  cloudwatch get-metric-data

LocalStack passthrough examples:
  foxtail s3 ls
  foxtail sqs list-queues
  foxtail ec2 describe-instances

Examples:
  Generate an idle-heavy dataset:
    foxtail gen --scenario idle-heavy --prune

  Start the service:
    foxtail serve --port 8080

  Foxtail-routed Cost Explorer:
    foxtail ce get-cost-and-usage --time-period Start=2026-03-01,End=2026-03-11 --granularity DAILY --metrics UnblendedCost

  Foxtail-routed CloudWatch:
    foxtail cloudwatch list-metrics --namespace AWS/EC2 --metric-name CPUUtilization

  LocalStack passthrough:
    foxtail ec2 describe-instances

  Explain the routing decision:
    foxtail --debug-routing s3 ls
    foxtail --debug-routing ce get-cost-and-usage --time-period Start=2026-03-01,End=2026-03-11 --granularity DAILY --metrics UnblendedCost
"
    )
}

pub fn version_text() -> &'static str {
    VERSION
}

pub fn render_debug_line(invocation: &Invocation) -> String {
    let args = invocation
        .args
        .iter()
        .map(|arg| shell_escape(arg))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "[foxtail] backend={} command={} {}",
        invocation.backend, invocation.program, args
    )
}

fn shell_escape(value: &OsStr) -> String {
    let text = value.to_string_lossy();
    if text
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | ':' | '.' | '_' | '-'))
    {
        text.into_owned()
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_wrapper_flags_before_forwarding() {
        let parsed = parse_cli_args(os(&[
            "--debug-routing",
            "--database-url",
            "sqlite:/tmp/foxtail.db",
            "--foxtail-endpoint",
            "http://127.0.0.1:19090",
            "ce",
            "get-cost-and-usage",
        ]))
        .unwrap();

        assert!(parsed.config.debug_routing);
        assert_eq!(parsed.config.database_url, "sqlite:/tmp/foxtail.db");
        assert_eq!(parsed.config.foxtail_endpoint, "http://127.0.0.1:19090");
        assert_eq!(parsed.forwarded_args, os(&["ce", "get-cost-and-usage"]));
        assert_eq!(parsed.mode, RunMode::Execute);
    }

    #[test]
    fn classifies_service_after_global_flag_values() {
        let target = classify_target(&os(&[
            "--profile",
            "dev",
            "--region",
            "us-east-1",
            "ce",
            "get-cost-and-usage",
        ]))
        .unwrap();

        assert_eq!(target.service, "ce");
        assert_eq!(target.operation, "get-cost-and-usage");
    }

    #[test]
    fn routes_supported_finops_commands_to_foxtail() {
        let parsed = ParsedCli {
            config: Config::default(),
            forwarded_args: os(&["cloudwatch", "list-metrics"]),
            mode: RunMode::Execute,
        };

        let invocation = build_invocation(&parsed);

        assert_eq!(invocation.backend, Backend::Aws);
        assert_eq!(invocation.program, DEFAULT_AWS_BIN);
        assert_eq!(
            invocation.args,
            os(&[
                "--endpoint-url",
                DEFAULT_FOXTAIL_ENDPOINT,
                "cloudwatch",
                "list-metrics"
            ])
        );
    }

    #[test]
    fn leaves_passthrough_commands_on_awslocal() {
        let parsed = ParsedCli {
            config: Config::default(),
            forwarded_args: os(&["s3", "ls"]),
            mode: RunMode::Execute,
        };

        let invocation = build_invocation(&parsed);

        assert_eq!(invocation.backend, Backend::Awslocal);
        assert_eq!(invocation.program, DEFAULT_AWSLOCAL_BIN);
        assert_eq!(invocation.args, os(&["s3", "ls"]));
    }

    #[test]
    fn respects_explicit_endpoint_for_routed_command() {
        let parsed = ParsedCli {
            config: Config::default(),
            forwarded_args: os(&[
                "--endpoint-url",
                "http://example.test",
                "ce",
                "get-cost-and-usage",
            ]),
            mode: RunMode::Execute,
        };

        let invocation = build_invocation(&parsed);

        assert_eq!(invocation.backend, Backend::Aws);
        assert_eq!(
            invocation.args,
            os(&[
                "--endpoint-url",
                "http://example.test",
                "ce",
                "get-cost-and-usage"
            ])
        );
    }

    #[test]
    fn help_text_is_agent_friendly_and_explicit_about_routing() {
        let help = help_text();

        assert!(help.contains("Native commands:"));
        assert!(help.contains("foxtail gen --scenario idle-heavy --prune"));
        assert!(help.contains("foxtail serve --port 8080"));
        assert!(help.contains("Routing rules:"));
        assert!(help.contains("If `(service, operation)` is in the Foxtail support table"));
        assert!(help.contains("Otherwise foxtail runs:"));
        assert!(help.contains("LocalStack passthrough examples:"));
        assert!(help.contains("Explain the routing decision:"));
        assert!(help.contains("ce get-cost-and-usage"));
        assert!(help.contains("ec2 describe-instances"));
    }
}

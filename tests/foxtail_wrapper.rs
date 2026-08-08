use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_temp_dir(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("foxtail-{name}-{nonce}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_stub(path: &Path, log_path: &Path, exit_code: i32) {
    let script = format!(
        "#!/usr/bin/env bash\nset -euo pipefail\nprintf '%s\\n' \"$0\" \"$@\" > \"{}\"\nexit {}\n",
        log_path.display(),
        exit_code
    );
    fs::write(path, script).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

#[test]
fn routes_cost_explorer_commands_to_aws_with_endpoint() {
    let temp_dir = unique_temp_dir("route-aws");
    let aws_log = temp_dir.join("aws.log");
    let awslocal_log = temp_dir.join("awslocal.log");
    let aws_bin = temp_dir.join("aws");
    let awslocal_bin = temp_dir.join("awslocal");

    write_stub(&aws_bin, &aws_log, 0);
    write_stub(&awslocal_bin, &awslocal_log, 0);

    let status = Command::new(env!("CARGO_BIN_EXE_foxtail"))
        .arg("--aws-bin")
        .arg(&aws_bin)
        .arg("--awslocal-bin")
        .arg(&awslocal_bin)
        .arg("ce")
        .arg("get-cost-and-usage")
        .status()
        .unwrap();

    assert!(status.success());
    assert!(aws_log.exists());
    assert!(!awslocal_log.exists());

    let log = fs::read_to_string(aws_log).unwrap();
    assert!(log.contains("--endpoint-url"));
    assert!(log.contains("http://127.0.0.1:8080"));
    assert!(log.contains("ce"));
    assert!(log.contains("get-cost-and-usage"));

    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn routes_ec2_describe_instances_to_aws_with_endpoint() {
    let temp_dir = unique_temp_dir("route-ec2");
    let aws_log = temp_dir.join("aws.log");
    let awslocal_log = temp_dir.join("awslocal.log");
    let aws_bin = temp_dir.join("aws");
    let awslocal_bin = temp_dir.join("awslocal");

    write_stub(&aws_bin, &aws_log, 0);
    write_stub(&awslocal_bin, &awslocal_log, 0);

    let status = Command::new(env!("CARGO_BIN_EXE_foxtail"))
        .arg("--aws-bin")
        .arg(&aws_bin)
        .arg("--awslocal-bin")
        .arg(&awslocal_bin)
        .arg("ec2")
        .arg("describe-instances")
        .status()
        .unwrap();

    assert!(status.success());
    assert!(aws_log.exists());
    assert!(!awslocal_log.exists());
    let log = fs::read_to_string(aws_log).unwrap();
    assert!(log.contains("--endpoint-url"));
    assert!(log.contains("http://127.0.0.1:8080"));
    assert!(log.contains("ec2"));
    assert!(log.contains("describe-instances"));

    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn routes_passthrough_commands_to_awslocal() {
    let temp_dir = unique_temp_dir("route-awslocal");
    let aws_log = temp_dir.join("aws.log");
    let awslocal_log = temp_dir.join("awslocal.log");
    let aws_bin = temp_dir.join("aws");
    let awslocal_bin = temp_dir.join("awslocal");

    write_stub(&aws_bin, &aws_log, 0);
    write_stub(&awslocal_bin, &awslocal_log, 0);

    let status = Command::new(env!("CARGO_BIN_EXE_foxtail"))
        .arg("--aws-bin")
        .arg(&aws_bin)
        .arg("--awslocal-bin")
        .arg(&awslocal_bin)
        .arg("s3")
        .arg("ls")
        .status()
        .unwrap();

    assert!(status.success());
    assert!(!aws_log.exists());
    assert!(awslocal_log.exists());

    let log = fs::read_to_string(awslocal_log).unwrap();
    assert!(log.contains("s3"));
    assert!(log.contains("ls"));

    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn propagates_subprocess_exit_code() {
    let temp_dir = unique_temp_dir("exit-code");
    let aws_log = temp_dir.join("aws.log");
    let awslocal_log = temp_dir.join("awslocal.log");
    let aws_bin = temp_dir.join("aws");
    let awslocal_bin = temp_dir.join("awslocal");

    write_stub(&aws_bin, &aws_log, 17);
    write_stub(&awslocal_bin, &awslocal_log, 0);

    let status = Command::new(env!("CARGO_BIN_EXE_foxtail"))
        .arg("--aws-bin")
        .arg(&aws_bin)
        .arg("--awslocal-bin")
        .arg(&awslocal_bin)
        .arg("cloudwatch")
        .arg("list-metrics")
        .status()
        .unwrap();

    assert_eq!(status.code(), Some(17));
    assert!(aws_log.exists());
    assert!(!awslocal_log.exists());

    fs::remove_dir_all(temp_dir).unwrap();
}

//! Integration test for the `calibrate` binary's `--dry-run` mode
//! (spec FR-17, AC-19): validates `golden/*.yaml` without any network call.

use std::process::{Command, Output};

fn run_calibrate_dry_run() -> Output {
  Command::new(env!("CARGO_BIN_EXE_calibrate"))
    .arg("--dry-run")
    .output()
    .expect("failed to run the calibrate binary")
}

#[test]
fn dry_run_validates_the_golden_set_without_network() {
  let output = run_calibrate_dry_run();
  assert!(
    output.status.success(),
    "expected exit 0 for --dry-run, got {:?}\nstdout: {}\nstderr: {}",
    output.status.code(),
    String::from_utf8_lossy(&output.stdout),
    String::from_utf8_lossy(&output.stderr),
  );
  let stdout = String::from_utf8_lossy(&output.stdout);
  assert!(stdout.contains("golden case(s) valid"), "stdout was: {stdout}");
  assert!(!stdout.contains("validation error"), "stdout was: {stdout}");
}

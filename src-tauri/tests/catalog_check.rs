//! Integration test for the `catalog-check` binary (spec FR-22, AC-2).

use std::process::{Command, Output};

fn run_catalog_check(skill_path: &str) -> Output {
  Command::new(env!("CARGO_BIN_EXE_catalog-check"))
    .arg(skill_path)
    .output()
    .expect("failed to run the catalog-check binary")
}

#[test]
fn no_drift_against_the_current_skill() {
  let output = run_catalog_check("tests/fixtures/skill/SKILL.md");
  assert!(
    output.status.success(),
    "expected exit 0 against the unmodified skill, got {:?}\nstdout: {}\nstderr: {}",
    output.status.code(),
    String::from_utf8_lossy(&output.stdout),
    String::from_utf8_lossy(&output.stderr),
  );
}

#[test]
fn drift_detected_against_the_drifted_skill() {
  let output = run_catalog_check("tests/fixtures/skill/SKILL-drifted.md");
  assert_eq!(
    output.status.code(),
    Some(1),
    "expected exit 1 (drift) against the drifted skill, got {:?}\nstdout: {}\nstderr: {}",
    output.status.code(),
    String::from_utf8_lossy(&output.stdout),
    String::from_utf8_lossy(&output.stderr),
  );
  let stdout = String::from_utf8_lossy(&output.stdout);
  assert!(stdout.contains("Testing (.NET)"), "stdout was: {stdout}");
  assert!(stdout.contains("Performance Testing"), "stdout was: {stdout}");
}

#[test]
fn usage_error_exits_2() {
  let output = run_catalog_check("tests/fixtures/skill/does-not-exist.md");
  assert_eq!(output.status.code(), Some(2));
}

//! `cargo run --bin catalog-check -- <path/to/SKILL.md>` (spec FR-22, AC-2).
//!
//! Parses the skill's "Quick Reference: Technology Selection Matrix" table
//! and diffs it against the catalogue bundled into this binary, reporting
//! rows added, rows removed, and cells changed. Exit 0 when identical, 1 on
//! drift, 2 on a usage or parse error.

use std::env;
use std::fs;
use std::process::ExitCode;

use bistec_architect::catalog::{diff_against_skill, Catalog};

fn main() -> ExitCode {
  let args: Vec<String> = env::args().collect();
  let Some(skill_path) = args.get(1) else {
    eprintln!("usage: catalog-check <path/to/SKILL.md>");
    return ExitCode::from(2);
  };

  let skill_md = match fs::read_to_string(skill_path) {
    Ok(text) => text,
    Err(err) => {
      eprintln!("catalog-check: failed to read {skill_path}: {err}");
      return ExitCode::from(2);
    }
  };

  let catalog = match Catalog::bundled() {
    Ok(catalog) => catalog,
    Err(err) => {
      eprintln!("catalog-check: failed to load bundled catalogue: {err}");
      return ExitCode::from(2);
    }
  };

  let drifts = match diff_against_skill(&catalog, &skill_md) {
    Ok(drifts) => drifts,
    Err(err) => {
      eprintln!("catalog-check: failed to parse {skill_path}: {err}");
      return ExitCode::from(2);
    }
  };

  if drifts.is_empty() {
    let matrix_rows = catalog.types.iter().filter(|t| t.matrix.is_some()).count();
    println!("catalog-check: no drift ({matrix_rows} matrix rows checked against {skill_path})");
    return ExitCode::SUCCESS;
  }

  println!("catalog-check: {} drift(s) found against {skill_path}", drifts.len());
  for drift in &drifts {
    println!("  - {drift}");
  }
  ExitCode::from(1)
}

//! Страж нормативного реестра: схема записей и свежесть индекса.
//!
//! Реестр описан в `spec/arch/README.md`. Проверка запускает `scripts/arch/registry.py`,
//! потому что разбор записей и порождение индекса живут там же, где формат.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn python() -> &'static str {
    if cfg!(windows) {
        "python"
    } else {
        "python3"
    }
}

#[test]
fn registry_records_match_the_published_schema_and_the_index_is_current() {
    let output = Command::new(python())
        .arg("scripts/arch/registry.py")
        .arg("--check")
        .current_dir(repo_root())
        .output()
        .expect("registry guard runs python3");

    assert!(
        output.status.success(),
        "spec/arch is invalid or its index is stale:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn every_decision_names_evidence_that_resolves() {
    let root = repo_root();
    let decisions = root.join("spec/arch/decisions");
    let mut unresolved = Vec::new();

    for entry in std::fs::read_dir(&decisions).expect("decisions directory is readable") {
        let path = entry.expect("directory entry").path();
        if path.extension().and_then(|value| value.to_str()) != Some("md") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("record is readable");
        let Some(line) = text.lines().find(|line| line.starts_with("realized: ")) else {
            unresolved.push(format!("{}: no realized prop", path.display()));
            continue;
        };
        let value = line.trim_start_matches("realized: ").trim();
        if value == "null" {
            continue;
        }
        let (file, name) = match value.split_once("::") {
            Some((file, name)) => (file, Some(name)),
            None => (value, None),
        };
        let evidence = root.join(file);
        if !evidence.is_file() {
            unresolved.push(format!("{}: missing evidence {file}", path.display()));
            continue;
        }
        if let Some(name) = name {
            let body = std::fs::read_to_string(&evidence).expect("evidence is readable");
            if !body.contains(&format!("fn {name}(")) {
                unresolved.push(format!("{}: {file} has no test {name}", path.display()));
            }
        }
    }

    assert!(
        unresolved.is_empty(),
        "decisions cite evidence that does not exist:\n{}",
        unresolved.join("\n")
    );
}

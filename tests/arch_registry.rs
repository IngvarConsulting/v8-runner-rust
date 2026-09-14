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

fn evidence_entries(value: &str) -> Vec<String> {
    let trimmed = value.trim();
    if let Some(inner) = trimmed
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
    {
        return inner
            .split(',')
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect();
    }
    vec![trimmed.to_string()]
}

fn unresolved_evidence(root: &std::path::Path, dir: &str, prop: &str) -> Vec<String> {
    let mut unresolved = Vec::new();
    let base = root.join(dir);
    for entry in std::fs::read_dir(&base).expect("registry directory is readable") {
        let path = entry.expect("directory entry").path();
        if path.extension().and_then(|value| value.to_str()) != Some("md") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("record is readable");
        let prefix = format!("{prop}: ");
        let Some(line) = text.lines().find(|line| line.starts_with(&prefix)) else {
            unresolved.push(format!("{}: no {prop} prop", path.display()));
            continue;
        };
        let value = line.trim_start_matches(&prefix).trim();
        if value == "null" {
            continue;
        }
        for item in evidence_entries(value) {
            let (file, name) = match item.split_once("::") {
                Some((file, name)) => (file, Some(name)),
                None => (item.as_str(), None),
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
    }
    unresolved
}

/// Правило со `status: planned` обязано объявлять отсутствие проверки полем
/// `check: null`, а действующее — называть её. Пустое поле у действующего правила
/// и названная проверка у запланированного одинаково прячут состояние долга.
#[test]
fn planned_rules_declare_a_missing_check() {
    let root = repo_root();
    let mut wrong = Vec::new();

    for dir in ["spec/arch/invariants", "spec/arch/contracts"] {
        let base = root.join(dir);
        for entry in std::fs::read_dir(&base).expect("registry directory is readable") {
            let path = entry.expect("directory entry").path();
            if path.extension().and_then(|value| value.to_str()) != Some("md") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("record is readable");
            let status = text
                .lines()
                .find_map(|line| line.strip_prefix("status: "))
                .unwrap_or_default()
                .trim()
                .to_string();
            let check = text
                .lines()
                .find_map(|line| line.strip_prefix("check: "))
                .unwrap_or_default()
                .trim()
                .to_string();

            match status.as_str() {
                "planned" if check != "null" => wrong.push(format!(
                    "{}: planned rule must declare `check: null`, found {check}",
                    path.display()
                )),
                "active" if check == "null" || check.is_empty() => wrong.push(format!(
                    "{}: active rule must name its falsifier",
                    path.display()
                )),
                _ => {}
            }
        }
    }

    assert!(
        wrong.is_empty(),
        "rules hide whether their falsifier exists:\n{}",
        wrong.join("\n")
    );
}

#[test]
fn every_rule_names_a_falsifier_that_exists() {
    let root = repo_root();
    let mut unresolved = unresolved_evidence(&root, "spec/arch/invariants", "check");
    unresolved.extend(unresolved_evidence(&root, "spec/arch/contracts", "check"));

    assert!(
        unresolved.is_empty(),
        "rules cite checks that do not exist:\n{}",
        unresolved.join("\n")
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

/// Раздел «Пример» у контракта — не проза, а проверяемый экземпляр формы.
///
/// Если артефакт контракта — схема, пример обязан её пройти; если артефакт не схема,
/// а закреплённый документ, пример обязан быть его фрагментом. Иначе пример живёт своей
/// жизнью и через два изменения формы врёт читателю ровно там, где тот ему верит.
#[test]
fn every_contract_shows_an_example_checked_against_its_form() {
    let root = repo_root();
    let contracts = root.join("spec/arch/contracts");
    let mut wrong = Vec::new();

    for entry in std::fs::read_dir(&contracts).expect("contracts directory is readable") {
        let path = entry.expect("directory entry").path();
        if path.extension().and_then(|value| value.to_str()) != Some("md") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("record is readable");
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_owned();

        let Some(artifact) = text
            .lines()
            .find_map(|line| line.strip_prefix("artifact: "))
            .map(|value| value.trim().to_owned())
        else {
            wrong.push(format!("{name}: no artifact prop"));
            continue;
        };
        let Some((language, example)) = example_block(&text) else {
            wrong.push(format!("{name}: no example block"));
            continue;
        };

        let artifact_text = match std::fs::read_to_string(root.join(&artifact)) {
            Ok(text) => text,
            Err(error) => {
                wrong.push(format!(
                    "{name}: artifact {artifact} is unreadable: {error}"
                ));
                continue;
            }
        };
        let artifact_value: serde_json::Value = match serde_json::from_str(&artifact_text) {
            Ok(value) => value,
            Err(error) => {
                wrong.push(format!("{name}: artifact {artifact} is not json: {error}"));
                continue;
            }
        };

        if let Some(line_kinds) = artifact_value
            .get("line_kinds")
            .and_then(serde_json::Value::as_object)
        {
            // Артефакт-грамматика описывает не документ, а строки. Пример к нему —
            // кусок настоящего вывода, и проверяется он построчно.
            let patterns: Vec<regex::Regex> = line_kinds
                .values()
                .filter_map(|kind| kind.get("pattern").and_then(serde_json::Value::as_str))
                .map(|pattern| regex::Regex::new(pattern).expect("kind pattern compiles"))
                .collect();
            for line in example.lines().filter(|line| !line.trim().is_empty()) {
                if !patterns.iter().any(|pattern| pattern.is_match(line)) {
                    wrong.push(format!(
                        "{name}: example line matches no kind of {artifact}: {line:?}"
                    ));
                }
            }
        } else if artifact_value.get("$schema").is_some() {
            let Some(parsed) = parse_example(&language, &example, &name, &mut wrong) else {
                continue;
            };
            let validator = match jsonschema::validator_for(&artifact_value) {
                Ok(validator) => validator,
                Err(error) => {
                    wrong.push(format!(
                        "{name}: artifact {artifact} is not a schema: {error}"
                    ));
                    continue;
                }
            };
            let errors: Vec<String> = validator
                .iter_errors(&parsed)
                .map(|error| format!("{} at {}", error, error.instance_path))
                .collect();
            if !errors.is_empty() {
                wrong.push(format!(
                    "{name}: example fails its own form {artifact}:\n{}",
                    errors.join("\n")
                ));
            }
        } else {
            let Some(parsed) = parse_example(&language, &example, &name, &mut wrong) else {
                continue;
            };
            if !contains(&artifact_value, &parsed) {
                wrong.push(format!(
                    "{name}: example is not a fragment of the pinned {artifact}"
                ));
            }
        }
    }

    assert!(
        wrong.is_empty(),
        "contract examples are prose, not pinned form:\n{}",
        wrong.join("\n")
    );
}

fn parse_example(
    language: &str,
    example: &str,
    name: &str,
    wrong: &mut Vec<String>,
) -> Option<serde_json::Value> {
    let parsed = match language {
        "yaml" => serde_yaml::from_str::<serde_json::Value>(example).map_err(|e| e.to_string()),
        _ => serde_json::from_str::<serde_json::Value>(example).map_err(|e| e.to_string()),
    };
    match parsed {
        Ok(value) => Some(value),
        Err(error) => {
            wrong.push(format!("{name}: example is not valid {language}: {error}"));
            None
        }
    }
}

/// Язык и тело первого блока кода в разделе «Пример».
fn example_block(text: &str) -> Option<(String, String)> {
    let heading = text.find("\n## Пример\n")? + "\n## Пример\n".len();
    let rest = &text[heading..];
    let open = rest.find("```")? + 3;
    let after_open = &rest[open..];
    let newline = after_open.find('\n')?;
    let language = after_open[..newline].trim().to_owned();
    let body = &after_open[newline + 1..];
    let close = body.find("\n```")?;
    Some((language, body[..close + 1].to_owned()))
}

/// Проверяет, что `fragment` целиком встречается в `whole`.
///
/// У объекта сверяются только названные фрагментом ключи, у всего остального —
/// равенство. Так пример показывает одну запись закреплённого документа, не переписывая
/// документ целиком.
fn contains(whole: &serde_json::Value, fragment: &serde_json::Value) -> bool {
    match (whole, fragment) {
        (serde_json::Value::Object(whole), serde_json::Value::Object(fragment)) => fragment
            .iter()
            .all(|(key, value)| whole.get(key).is_some_and(|found| contains(found, value))),
        _ => whole == fragment,
    }
}

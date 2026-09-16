//! Страж нормативного реестра: схема записей и свежесть индекса.
//!
//! Реестр описан в `spec/arch/README.md`. Проверка запускает `scripts/arch/registry.py`,
//! потому что разбор записей и порождение индекса живут там же, где формат.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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

/// Прежний слой заморожен в `spec/archive/`, и номер ADR больше ничего не адресует: у
/// каждой записи ровно один владелец в реестре по таблице судьбы. Ссылка по номеру вне
/// архива — это ссылка в никуда, и она не должна вернуться ни в код, ни в документы.
#[test]
fn old_adr_numbers_do_not_return_outside_the_archive() {
    let root = repo_root();
    let number = regex::Regex::new(r"ADR-\d{4}").expect("regex");
    // Решение о переезде называет прежний слой по его же номерам — это его предмет.
    let allowed = [
        root.join("spec/archive"),
        root.join("spec/arch/decisions/2026-09-14-spec-registry-reset.md"),
    ];
    let mut offenders = Vec::new();
    let mut pending: Vec<PathBuf> = ["src", "tests", "docs", "spec", "scripts"]
        .iter()
        .map(|dir| root.join(dir))
        .collect();
    for entry in std::fs::read_dir(&root).expect("repo root is readable") {
        let path = entry.expect("directory entry").path();
        if path.extension().and_then(|value| value.to_str()) == Some("md") {
            pending.push(path);
        }
    }
    while let Some(path) = pending.pop() {
        if allowed.iter().any(|prefix| path.starts_with(prefix)) {
            continue;
        }
        if path.is_dir() {
            for entry in std::fs::read_dir(&path).expect("directory is readable") {
                pending.push(entry.expect("directory entry").path());
            }
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (index, line) in text.lines().enumerate() {
            if let Some(found) = number.find(line) {
                offenders.push(format!(
                    "{}:{}: {}",
                    path.strip_prefix(&root).unwrap_or(&path).display(),
                    index + 1,
                    found.as_str()
                ));
            }
        }
    }
    offenders.sort();
    assert!(
        offenders.is_empty(),
        "old ADR numbers address nothing; name the owner from spec/archive/FATE.md instead:\n{}",
        offenders.join("\n")
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
        // Решение может держаться несколькими свидетельствами: список в квадратных
        // скобках, как и у `check` правил.
        let entries = value
            .trim_start_matches('[')
            .trim_end_matches(']')
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty());
        for entry in entries {
            let (file, name) = match entry.split_once("::") {
                Some((file, name)) => (file, Some(name)),
                None => (entry, None),
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

    assert!(
        unresolved.is_empty(),
        "decisions cite evidence that does not exist:\n{}",
        unresolved.join("\n")
    );
}

/// Пробник: прогоняет выдуманный мини-реестр через ту же `validation_errors`,
/// которой пользуется `--check`, и отдаёт её отказы списком.
///
/// Скрипт едет на stdin, а не лежит файлом рядом со `scripts/arch/registry.py`:
/// схема записей живёт в одном месте, и второй её носитель устарел бы первым.
/// Модуль смотрит на выдуманный корень обоими концами — `records(root)` читает
/// оттуда, `ARCH_ROOT` оттуда же считает путь записи для текста отказа.
const REGISTRY_PROBE: &str = r#"
import importlib.util
import json
import pathlib
import sys

spec = importlib.util.spec_from_file_location("registry", sys.argv[1])
registry = importlib.util.module_from_spec(spec)
# Запись — dataclass с отложенными аннотациями, и разрешает она их через
# `sys.modules`: модуль, загруженный мимо него, разваливается на `@dataclass`.
sys.modules[spec.name] = registry
spec.loader.exec_module(registry)

root = pathlib.Path(sys.argv[2]).resolve()
registry.ARCH_ROOT = root
print(json.dumps(registry.validation_errors(registry.records(root))))
"#;

/// Отказы реестра о мини-реестре, разложенном в `root`.
fn python_validation_errors(root: &Path) -> Vec<String> {
    let mut probe = Command::new(python())
        .arg("-")
        .arg("scripts/arch/registry.py")
        .arg(root)
        .current_dir(repo_root())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("registry probe runs python3");
    probe
        .stdin
        .take()
        .expect("probe takes a script on stdin")
        .write_all(REGISTRY_PROBE.as_bytes())
        .expect("probe script is written");
    let output = probe.wait_with_output().expect("probe finishes");
    assert!(
        output.status.success(),
        "registry probe failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("probe prints a json list of errors")
}

/// Выдуманная запись: символ, пропы и заголовок — ровно столько, сколько нужно
/// схеме, чтобы отказать по одной названной причине, а не по пяти сразу.
fn fabricated_record(symbol: &str, props: &str) -> String {
    format!("---\nid: {symbol}\n{props}---\n\n# Выдуманная запись\n")
}

/// Мини-реестр в раскладке `spec/arch`: каталог вида записи, файл на запись.
///
/// Записи — решения и правила без формы: проп `artifact` контракта реестр ищет от
/// настоящего корня репозитория, и выдуманный контракт дал бы лишний отказ о нём.
fn fabricated(records: &[(&str, String)]) -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("temporary registry");
    for (relative, text) in records {
        let path = root.path().join(relative);
        std::fs::create_dir_all(path.parent().expect("record lies in a registry directory"))
            .expect("registry directory is created");
        std::fs::write(&path, text).expect("record is written");
    }
    root
}

/// Мини-реестр обязан дать ровно один отказ, и отказ обязан назвать предмет.
///
/// «Ровно один» держит фикстуру честной: отказ по другой причине не сойдёт за
/// проверяемый, и испорченная заготовка записи будет видна сразу.
fn sole_error(records: &[(&str, String)], names: &str) {
    let root = fabricated(records);
    let errors = python_validation_errors(root.path());
    assert_eq!(
        errors.len(),
        1,
        "fixture must fail for exactly one reason:\n{}",
        errors.join("\n")
    );
    assert!(
        errors[0].contains(names),
        "error does not name {names}:\n{}",
        errors[0]
    );
}

/// Символ, которым решение называет чужую запись, обязан в эту запись разрешаться.
///
/// `establishes` ведёт к правилу, `supersedes` и `superseded-by` — к решению.
/// Ненайденный символ публикует обещание, за которым нет ни записи, ни проверки,
/// поэтому отказ называет ещё и путь, по которому запись надо написать.
#[test]
fn every_symbol_a_decision_names_resolves_to_a_record() {
    const NEWER: &str = "DEC.2026-01-02.A-FABRICATED-SUCCESSOR";
    const OLDER: &str = "DEC.2026-01-01.A-FABRICATED-DECISION";
    const RULE: &str = "INV.DOCS.A-FABRICATED-RULE";
    const NEWER_FILE: &str = "decisions/2026-01-02-a-fabricated-successor.md";
    const OLDER_FILE: &str = "decisions/2026-01-01-a-fabricated-decision.md";
    const RULE_FILE: &str = "invariants/INV.DOCS.A-FABRICATED-RULE.md";
    const SOUND_DECISION: &str = "status: active\ngoverns: process\nrealized: tests/probe.rs::a\n";
    const SOUND_RULE: &str =
        "status: active\ngoverns: process\ncheck: tests/probe.rs::a\nscope: [docs]\n";

    let successor = |extra: &str| fabricated_record(NEWER, &format!("{SOUND_DECISION}{extra}"));
    let rule = || fabricated_record(RULE, &format!("{SOUND_RULE}decision: {NEWER}\n"));

    // Здоровая фикстура: все три пропа заполнены и разрешаются. Без неё «ровно
    // один отказ» ниже мог бы оказаться совпадением, а не проверкой.
    let sound = fabricated(&[
        (
            NEWER_FILE,
            successor(&format!("supersedes: [{OLDER}]\nestablishes: [{RULE}]\n")),
        ),
        (
            OLDER_FILE,
            fabricated_record(
                OLDER,
                &format!(
                    "status: superseded\ngoverns: process\nrealized: null\nsuperseded-by: {NEWER}\nestablishes: []\n"
                ),
            ),
        ),
        (RULE_FILE, rule()),
    ]);
    let errors = python_validation_errors(sound.path());
    assert!(
        errors.is_empty(),
        "a registry whose symbols all resolve must pass:\n{}",
        errors.join("\n")
    );

    // Заменённое решение отвечает за свой `establishes` только историей: правило,
    // выведенное из обращения преемником, файла уже не имеет, и требовать его
    // значило бы запереть судьбу `retired` из `spec/archive/FATE.md` навсегда.
    let retired = fabricated(&[
        (
            OLDER_FILE,
            fabricated_record(
                OLDER,
                &format!(
                    "status: superseded\ngoverns: process\nrealized: null\nsuperseded-by: {NEWER}\nestablishes: [INV.DOCS.A-RETIRED-RULE]\n"
                ),
            ),
        ),
        (
            NEWER_FILE,
            successor(&format!("supersedes: [{OLDER}]\nestablishes: []\n")),
        ),
    ]);
    let errors = python_validation_errors(retired.path());
    assert!(
        errors.is_empty(),
        "a superseded decision keeps its list as history:\n{}",
        errors.join("\n")
    );

    // `establishes` называет правило, записи которого нет: отказ называет файл,
    // который автору осталось написать.
    sole_error(
        &[(
            NEWER_FILE,
            successor("establishes: [INV.DOCS.A-RULE-NOBODY-WROTE]\n"),
        )],
        "invariants/INV.DOCS.A-RULE-NOBODY-WROTE.md",
    );

    // `establishes` называет решение: правило выводится из решения, а не наоборот.
    sole_error(
        &[
            (NEWER_FILE, successor(&format!("establishes: [{OLDER}]\n"))),
            (
                OLDER_FILE,
                fabricated_record(OLDER, &format!("{SOUND_DECISION}establishes: []\n")),
            ),
        ],
        "names the decision DEC.2026-01-01.A-FABRICATED-DECISION where a rule is required",
    );

    // Символ не из реестра не разрешается ни во что и файла не подсказывает.
    sole_error(
        &[(NEWER_FILE, successor("establishes: [mcp-tools]\n"))],
        "names mcp-tools, which is not a rule symbol",
    );

    // `supersedes` называет решение, которого нет: имя решения выводит путь файла.
    sole_error(
        &[(
            NEWER_FILE,
            successor("supersedes: [DEC.2026-01-01.A-DECISION-NOBODY-WROTE]\n"),
        )],
        "decisions/2026-01-01-a-decision-nobody-wrote.md",
    );

    // `supersedes` называет правило: заменяют решение, а не выведенное из него.
    sole_error(
        &[
            (
                NEWER_FILE,
                successor(&format!("supersedes: [{RULE}]\nestablishes: [{RULE}]\n")),
            ),
            (RULE_FILE, rule()),
        ],
        "names the invariant INV.DOCS.A-FABRICATED-RULE where a decision is required",
    );

    // `superseded-by` — скаляр, и разрешается он так же, как список `supersedes`.
    sole_error(
        &[(
            OLDER_FILE,
            fabricated_record(
                OLDER,
                "status: superseded\ngoverns: process\nrealized: null\nsuperseded-by: DEC.2026-01-03.A-SUCCESSOR-NOBODY-WROTE\n",
            ),
        )],
        "decisions/2026-01-03-a-successor-nobody-wrote.md",
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

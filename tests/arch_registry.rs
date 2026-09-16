//! Страж нормативного реестра: схема записей и свежесть индекса.
//!
//! Реестр описан в `spec/arch/README.md`. Проверка запускает `scripts/arch/registry.py`,
//! потому что разбор записей и порождение индекса живут там же, где формат.

use std::io::Write;
use std::path::PathBuf;
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

/// Запись реестра как файл: каталог, имя файла, текст.
type RecordFile = (String, String, String);

const DECISION_FILE: &str = "2026-09-16-an-example-decision.md";
const DECISION_ID: &str = "DEC.2026-09-16.AN-EXAMPLE-DECISION";
const EVIDENCE: &str = "tests/arch_registry.rs::a_symbol_and_its_path_spell_each_other";

/// Решение, которое заводит правило фикстуры.
///
/// Запись не проверить по одному файлу: правило обязано сослаться на решение, а
/// решение — назвать правило в `establishes`. Поэтому фикстура здесь — не файл, а
/// маленький реестр целиком, и нарушение в нём ровно одно.
fn decision_file(name: &str, id: &str, establishes: &str) -> RecordFile {
    (
        "decisions".to_owned(),
        name.to_owned(),
        format!(
            "---\n\
             id: {id}\n\
             status: active\n\
             governs: process\n\
             realized: {EVIDENCE}\n\
             supersedes: []\n\
             superseded-by: null\n\
             establishes: [{establishes}]\n\
             ---\n\
             \n\
             # Решение\n"
        ),
    )
}

/// Инвариант, выведенный из этого решения.
fn rule_file(name: &str, id: &str, decision: &str) -> RecordFile {
    (
        "invariants".to_owned(),
        name.to_owned(),
        format!(
            "---\n\
             id: {id}\n\
             status: active\n\
             governs: process\n\
             decision: {decision}\n\
             check: {EVIDENCE}\n\
             scope: [docs]\n\
             ---\n\
             \n\
             # Правило\n"
        ),
    )
}

/// Контракт: та же запись, но с формой, закреплённой в файле, и с примером.
///
/// Он здесь не ради контрактов, а ради того, что префикс вида берётся из
/// `SYMBOL_PREFIX` по виду записи, а не зашит одним `INV.` на всех.
fn contract_file(name: &str, id: &str, decision: &str) -> RecordFile {
    (
        "contracts".to_owned(),
        name.to_owned(),
        format!(
            "---\n\
             id: {id}\n\
             status: active\n\
             governs: product\n\
             version: 1\n\
             decision: {decision}\n\
             producer: src/output/text.rs\n\
             artifact: docs/schemas/text-output.json\n\
             consumers: [cli]\n\
             check: {EVIDENCE}\n\
             scope: [wire]\n\
             ---\n\
             \n\
             # Контракт\n\
             \n\
             ## Пример\n\
             \n\
             ```json\n\
             {{}}\n\
             ```\n"
        ),
    )
}

/// Та же запись, но с переписанными полями.
///
/// Фальсификатор обязан отличаться от здоровой фикстуры ровно тем, что проверяет:
/// собранный заново, он разошёлся бы с ней и вторым полем, и нарушений в нём стало бы
/// два, а проба требует одного.
fn with_props(file: RecordFile, changes: &[(&str, &str)]) -> RecordFile {
    let (directory, name, text) = file;
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    for (key, value) in changes {
        let prefix = format!("{key}: ");
        let line = lines
            .iter_mut()
            .find(|line| line.starts_with(&prefix))
            .unwrap_or_else(|| panic!("fixture has no `{key}` prop to rewrite"));
        *line = format!("{prefix}{value}");
    }
    (directory, name, lines.join("\n") + "\n")
}

/// Мини-реестр как вход пробы: файлы и, если нужно, подменённый префикс вида.
fn registry_case(files: Vec<RecordFile>) -> serde_json::Value {
    serde_json::json!({ "files": files, "prefix": {} })
}

/// Тот же реестр, но вид записи назван другим префиксом — так, как это однажды и было.
fn registry_case_with_prefix(
    files: Vec<RecordFile>,
    kind: &str,
    prefix: &str,
) -> serde_json::Value {
    serde_json::json!({ "files": files, "prefix": { kind: prefix } })
}

/// Судит фикстуры тем же кодом, которым гейт `registry.py --check` судит реестр.
///
/// Фикстура на диск не кладётся, и причина — предмет одной из проверок ниже: файл с
/// базовым именем `CON` на Windows не создаётся, так что тест про имена, которые
/// Windows отвергает, был бы единственным, кто на Windows и падает. Запись собирает
/// `record_from` — та же сборка, что у обхода каталога, поэтому судится здесь ровно та
/// форма, которую гейт и получает; путь при этом остаётся именем, а не файлом.
const REGISTRY_PROBE: &str = r#"
import json, pathlib, sys

sys.dont_write_bytecode = True
sys.path.insert(0, "scripts/arch")
import registry

# Запись называет себя путём от корня реестра, и корень тут чисто именной: без этой
# подмены `Record.relative` меряет путь от настоящего spec/arch и падает на первой же
# найденной ошибке — там, где ошибку надо не поднять, а вернуть.
registry.ARCH_ROOT = pathlib.PurePosixPath("spec/arch")
prefixes = dict(registry.SYMBOL_PREFIX)

answer = []
for case in json.load(sys.stdin):
    registry.SYMBOL_PREFIX = {**prefixes, **case["prefix"]}
    found = [
        registry.record_from(
            registry.ARCH_ROOT / directory / name, text, registry.KIND_BY_DIR[directory]
        )
        for directory, name, text in case["files"]
    ]
    answer.append(registry.validation_errors(sorted(found, key=lambda record: record.id)))
json.dump(answer, sys.stdout, ensure_ascii=False)
"#;

/// Претензии `registry.py` к каждому мини-реестру, по порядку.
fn python_validation_errors(cases: &[serde_json::Value]) -> Vec<Vec<String>> {
    let mut probe = Command::new(python())
        .arg("-c")
        .arg(REGISTRY_PROBE)
        .current_dir(repo_root())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("registry guard runs python3");
    probe
        .stdin
        .take()
        .expect("probe takes its input on stdin")
        .write_all(&serde_json::to_vec(cases).expect("fixtures serialize"))
        .expect("probe reads its input");
    let output = probe.wait_with_output().expect("probe answers");

    assert!(
        output.status.success(),
        "registry.py cannot judge a record at all:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("probe answers json")
}

/// Одно нарушение на фикстуру: гейт обязан назвать именно его и больше ничего.
fn sole_error(name: &str, errors: &[String], expected: &str, wrong: &mut Vec<String>) {
    match errors {
        [only] if only.contains(expected) => {}
        _ => wrong.push(format!("{name}: expected `{expected}`, got {errors:?}")),
    }
}

/// Символ и путь восстанавливают друг друга — это обещание реестра, а не примета.
///
/// `spec/arch/README.md` обещает про `id`: «Совпадает с путём файла; по одному
/// восстанавливается другое». Обещание держит навигацию: по символу из чужого текста
/// открывают файл, не заглядывая в индекс. Обратный ход собирается из двух половин —
/// префикс вида называет каталог, остальное имя файла, — и обе обязаны сойтись.
/// Разойдись они, и ссылка по символу ведёт не в тот файл или никуда, а индекс подмену
/// повторяет: он порождается из тех же записей и потому с ними согласен.
#[test]
fn a_symbol_and_its_path_spell_each_other() {
    let sound = registry_case(vec![
        decision_file(DECISION_FILE, DECISION_ID, "INV.DOCS.EXAMPLE"),
        rule_file("INV.DOCS.EXAMPLE.md", "INV.DOCS.EXAMPLE", DECISION_ID),
    ]);
    let sound_contract = registry_case(vec![
        decision_file(DECISION_FILE, DECISION_ID, "CTR.WIRE.EXAMPLE"),
        contract_file("CTR.WIRE.EXAMPLE.md", "CTR.WIRE.EXAMPLE", DECISION_ID),
    ]);
    let rule_renamed = registry_case(vec![
        decision_file(DECISION_FILE, DECISION_ID, "INV.DOCS.OTHER"),
        rule_file("INV.DOCS.EXAMPLE.md", "INV.DOCS.OTHER", DECISION_ID),
    ]);
    let decision_renamed = registry_case(vec![
        decision_file(
            DECISION_FILE,
            "DEC.2026-09-16.SOMETHING-ELSE",
            "INV.DOCS.EXAMPLE",
        ),
        rule_file(
            "INV.DOCS.EXAMPLE.md",
            "INV.DOCS.EXAMPLE",
            "DEC.2026-09-16.SOMETHING-ELSE",
        ),
    ]);
    let decision_misfiled = registry_case(vec![
        decision_file("an-example-decision.md", DECISION_ID, "INV.DOCS.EXAMPLE"),
        rule_file("INV.DOCS.EXAMPLE.md", "INV.DOCS.EXAMPLE", DECISION_ID),
    ]);
    // Символ обещает каталог `contracts/`, а лежит запись в `invariants/`: по символу
    // её не найти, а два таких файла дали бы в индексе две строки на один символ.
    let rule_in_the_wrong_registry = registry_case(vec![
        decision_file(DECISION_FILE, DECISION_ID, "CTR.WIRE.EXAMPLE"),
        rule_file("CTR.WIRE.EXAMPLE.md", "CTR.WIRE.EXAMPLE", DECISION_ID),
    ]);

    let judged = python_validation_errors(&[
        sound,
        sound_contract,
        rule_renamed,
        decision_renamed,
        decision_misfiled,
        rule_in_the_wrong_registry,
    ]);
    let mut wrong = Vec::new();

    for (name, errors) in [
        ("a sound rule", &judged[0]),
        ("a sound contract", &judged[1]),
    ] {
        if !errors.is_empty() {
            wrong.push(format!("{name} must pass: {errors:?}"));
        }
    }
    sole_error(
        "a rule whose id is not its filename",
        &judged[2],
        "`id` must read `INV.DOCS.EXAMPLE`",
        &mut wrong,
    );
    sole_error(
        "a decision whose id is not its filename",
        &judged[3],
        &format!("`id` must read `{DECISION_ID}`"),
        &mut wrong,
    );
    sole_error(
        "a decision filed under a name that spells no symbol",
        &judged[4],
        "decisions/an-example-decision.md: filename must read",
        &mut wrong,
    );
    sole_error(
        "a rule whose symbol names another registry",
        &judged[5],
        "`id` must open with `INV.`",
        &mut wrong,
    );

    assert!(
        wrong.is_empty(),
        "the symbol and the path may drift apart:\n{}",
        wrong.join("\n")
    );
}

/// Имя записи — то, что git выкладывает на диск, и на Windows тоже.
///
/// `CON` был первым префиксом контрактов, и дерево переставало выкладываться на Windows
/// целиком: базовое имя из списка DOS-устройств система отказывается создавать с любым
/// расширением. Поэтому фикстура здесь и переименовывает вид записи — воспроизводится
/// ровно тот случай, а не выдуманный. Под нынешними префиксами проверка молчит всегда:
/// `DEC`, `INV` и `CTR` устройствами не зовутся. Это не делает её лишней — она сторожит
/// не запись, а нашу же константу, которую однажды уже так и меняли.
///
/// Запрет ровно такой, каким его ставит система, и обе границы здесь закреплены.
/// Смотрит он на базовое имя — то, что до первой точки, — поэтому `INV.DOCS.CON.md`
/// Windows создаёт и реестр принимает. Список кончается на `COM1`…`COM9`: `COM0`
/// система не резервирует. Строгость сверх системной заявляла бы правило шире того,
/// что проверено, — ровно та же ошибка, что и пропуск настоящего имени устройства.
#[test]
fn a_record_name_survives_a_windows_checkout() {
    let contracts_called_con = registry_case_with_prefix(
        vec![
            decision_file(DECISION_FILE, DECISION_ID, "CON.WIRE.EXAMPLE"),
            contract_file("CON.WIRE.EXAMPLE.md", "CON.WIRE.EXAMPLE", DECISION_ID),
        ],
        "contract",
        "CON",
    );
    let device_name_deeper = registry_case(vec![
        decision_file(DECISION_FILE, DECISION_ID, "INV.DOCS.CON"),
        rule_file("INV.DOCS.CON.md", "INV.DOCS.CON", DECISION_ID),
    ]);
    // Устройства нумеруются с единицы: `COM1` система резервирует, `COM0` — нет.
    let port_zero = registry_case_with_prefix(
        vec![
            decision_file(DECISION_FILE, DECISION_ID, "COM0.WIRE.EXAMPLE"),
            contract_file("COM0.WIRE.EXAMPLE.md", "COM0.WIRE.EXAMPLE", DECISION_ID),
        ],
        "contract",
        "COM0",
    );

    let judged = python_validation_errors(&[contracts_called_con, device_name_deeper, port_zero]);
    let mut wrong = Vec::new();

    sole_error(
        "a prefix that makes every record of its kind a device",
        &judged[0],
        "`CON` is a Windows device name",
        &mut wrong,
    );
    for (name, errors) in [
        ("a device name past the first dot", &judged[1]),
        ("a port number the system does not reserve", &judged[2]),
    ] {
        if !errors.is_empty() {
            wrong.push(format!("{name} is not a device: {errors:?}"));
        }
    }

    assert!(
        wrong.is_empty(),
        "the tree may grow a name Windows refuses to check out:\n{}",
        wrong.join("\n")
    );
}

/// Слово, по которому реестр судит, обязано быть из опубликованного перечня.
///
/// `status` — не колонка индекса, а условие: `planned` разрешает правилу не называть
/// фальсификатор, `superseded` — решению не предъявлять свидетельство, `active` требует
/// действующего решения под действующим правилом. Сверяется слово целиком, поэтому
/// промах мимо перечня не нарушает правил, а отключает проверку: правило со
/// `status: activ` под `planned`-решением до этой проверки проходило гейт молча —
/// объявляло себя действующим под непринятым решением, и это никого не касалось.
/// `governs` ничего не переключает и ломается иначе, но так же тихо: он уходит в
/// индекс как есть, и колонка перестаёт группироваться.
///
/// Перечень закрыт с обеих сторон, и фикстуры держат обе. Опубликованное значение
/// обязано проходить — включая `superseded`, которого нет ни в одной живой записи:
/// сузься перечень до двух слов, реестр бы этого не заметил. Неопубликованное обязано
/// отвергаться — и тогда, когда выглядит уместным: четвёртая ось заводится решением,
/// а не правкой одного поля.
#[test]
fn a_closed_field_admits_only_published_values() {
    let sound_rule = registry_case(vec![
        decision_file(DECISION_FILE, DECISION_ID, "INV.DOCS.EXAMPLE"),
        rule_file("INV.DOCS.EXAMPLE.md", "INV.DOCS.EXAMPLE", DECISION_ID),
    ]);
    // `governs: product` носит контракт фикстуры, `process` — правило и решение.
    let sound_contract = registry_case(vec![
        decision_file(DECISION_FILE, DECISION_ID, "CTR.WIRE.EXAMPLE"),
        contract_file("CTR.WIRE.EXAMPLE.md", "CTR.WIRE.EXAMPLE", DECISION_ID),
    ]);
    let planned_rule = registry_case(vec![
        decision_file(DECISION_FILE, DECISION_ID, "INV.DOCS.EXAMPLE"),
        with_props(
            rule_file("INV.DOCS.EXAMPLE.md", "INV.DOCS.EXAMPLE", DECISION_ID),
            &[("status", "planned"), ("check", "null")],
        ),
    ]);
    // Заменённое решение: правило под ним ещё не действует, иначе его поймала бы
    // ветка «действующее правило под недействующим решением».
    let superseded_decision = registry_case(vec![
        with_props(
            decision_file(DECISION_FILE, DECISION_ID, "INV.DOCS.EXAMPLE"),
            &[("status", "superseded"), ("realized", "null")],
        ),
        with_props(
            rule_file("INV.DOCS.EXAMPLE.md", "INV.DOCS.EXAMPLE", DECISION_ID),
            &[("status", "planned"), ("check", "null")],
        ),
    ]);
    let misspelled_governs = registry_case(vec![
        decision_file(DECISION_FILE, DECISION_ID, "INV.DOCS.EXAMPLE"),
        with_props(
            rule_file("INV.DOCS.EXAMPLE.md", "INV.DOCS.EXAMPLE", DECISION_ID),
            &[("governs", "proces")],
        ),
    ]);
    // Ровно тот случай, ради которого перечень и закрывается: решение не действует,
    // правило объявляет себя действующим, и одна буква прячет расхождение целиком.
    let misspelled_status_switches_a_check_off = registry_case(vec![
        with_props(
            decision_file(DECISION_FILE, DECISION_ID, "INV.DOCS.EXAMPLE"),
            &[("status", "planned"), ("realized", "null")],
        ),
        with_props(
            rule_file("INV.DOCS.EXAMPLE.md", "INV.DOCS.EXAMPLE", DECISION_ID),
            &[("status", "activ")],
        ),
    ]);
    let axis_nobody_published = registry_case(vec![
        with_props(
            decision_file(DECISION_FILE, DECISION_ID, "INV.DOCS.EXAMPLE"),
            &[("governs", "architecture")],
        ),
        rule_file("INV.DOCS.EXAMPLE.md", "INV.DOCS.EXAMPLE", DECISION_ID),
    ]);

    let judged = python_validation_errors(&[
        sound_rule,
        sound_contract,
        planned_rule,
        superseded_decision,
        misspelled_governs,
        misspelled_status_switches_a_check_off,
        axis_nobody_published,
    ]);
    let mut wrong = Vec::new();

    for (name, errors) in [
        ("active, process", &judged[0]),
        ("product on a contract", &judged[1]),
        ("planned", &judged[2]),
        ("superseded", &judged[3]),
    ] {
        if !errors.is_empty() {
            wrong.push(format!("published `{name}` must pass: {errors:?}"));
        }
    }
    sole_error(
        "a misspelled governs",
        &judged[4],
        "`governs` must be one of product, process; found `proces`",
        &mut wrong,
    );
    sole_error(
        "a misspelled status that switches a check off",
        &judged[5],
        "`status` must be one of active, planned, superseded; found `activ`",
        &mut wrong,
    );
    sole_error(
        "an axis nobody published",
        &judged[6],
        "`governs` must be one of product, process; found `architecture`",
        &mut wrong,
    );

    assert!(
        wrong.is_empty(),
        "a field the registry judges by may hold a word it does not know:\n{}",
        wrong.join("\n")
    );
}

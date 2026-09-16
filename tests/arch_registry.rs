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

    // Дата — часть символа, и пишется она теми же ASCII-цифрами, что и остальное имя.
    // `\d` принимает и арабо-индийские, а из них собирается символ, которого нет ни в
    // одной ссылке на решение: набрать его с клавиатуры и не выйдет.
    let decision_dated_in_other_digits = registry_case(vec![
        decision_file(
            "\u{664}\u{660}\u{662}\u{666}-\u{660}\u{669}-\u{661}\u{666}-an-example-decision.md",
            "DEC.\u{664}\u{660}\u{662}\u{666}-\u{660}\u{669}-\u{661}\u{666}.AN-EXAMPLE-DECISION",
            "INV.DOCS.EXAMPLE",
        ),
        rule_file(
            "INV.DOCS.EXAMPLE.md",
            "INV.DOCS.EXAMPLE",
            "DEC.\u{664}\u{660}\u{662}\u{666}-\u{660}\u{669}-\u{661}\u{666}.AN-EXAMPLE-DECISION",
        ),
    ]);

    let judged = python_validation_errors(&[
        sound,
        sound_contract,
        rule_renamed,
        decision_renamed,
        decision_misfiled,
        rule_in_the_wrong_registry,
        decision_dated_in_other_digits,
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
    sole_error(
        "a decision dated in digits no reference can spell",
        &judged[6],
        "filename must read",
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

const SUCCESSOR_FILE: &str = "2026-09-17-a-later-decision.md";
const SUCCESSOR_ID: &str = "DEC.2026-09-17.A-LATER-DECISION";

/// Та же запись, но одно поле переписано — или дописано, если его не было.
///
/// Фикстуры ниже отличаются от здоровой записи ровно одним полем: тем, о котором
/// проверка. Отдельный конструктор на каждую держал бы десяток почти одинаковых
/// заготовок, и нарушение в них терялось бы среди совпадений.
fn with_prop(file: RecordFile, key: &str, value: &str) -> RecordFile {
    let (directory, name, text) = file;
    // Двоеточие с пробелом отделяет ключ целиком: иначе `supersedes` переписывал бы
    // и `superseded-by`.
    let opening = format!("{key}: ");
    if text.lines().any(|line| line.starts_with(&opening)) {
        let rewritten = text
            .lines()
            .map(|line| {
                if line.starts_with(&opening) {
                    format!("{key}: {value}")
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        return (directory, name, rewritten + "\n");
    }
    let close = text
        .find("\n---\n")
        .expect("fixture opens with a front-matter block");
    let mut grown = text[..close].to_owned();
    grown.push_str(&format!("\n{key}: {value}"));
    grown.push_str(&text[close..]);
    (directory, name, grown)
}

/// Решение, которое ничего не заводит: предмет фикстур про замену — сама пара.
fn plain_decision(name: &str, id: &str) -> RecordFile {
    decision_file(name, id, "")
}

/// Символ называет ровно одну запись, иначе по нему открывается то одна, то другая.
///
/// Имя файла и префикс вида диктуют символ порознь, и вместе уникальности ещё не дают:
/// стоит двум видам назваться одним префиксом, и `INV.DOCS.EXAMPLE` лежит и в
/// `invariants/`, и в `contracts/`. Обе записи проходят все проверки имени, ссылка по
/// символу приводит к той, что победила в `by_id`, а индекс печатает на один символ две
/// строки — и обе выглядят правдой. Префикс контрактов уже меняли однажды, поэтому
/// фикстура меняет его, а не выдумывает случай.
#[test]
fn a_symbol_names_exactly_one_record() {
    let sound = registry_case(vec![
        decision_file(DECISION_FILE, DECISION_ID, "INV.DOCS.EXAMPLE"),
        rule_file("INV.DOCS.EXAMPLE.md", "INV.DOCS.EXAMPLE", DECISION_ID),
    ]);
    let two_kinds_one_prefix = registry_case_with_prefix(
        vec![
            decision_file(DECISION_FILE, DECISION_ID, "INV.DOCS.EXAMPLE"),
            rule_file("INV.DOCS.EXAMPLE.md", "INV.DOCS.EXAMPLE", DECISION_ID),
            contract_file("INV.DOCS.EXAMPLE.md", "INV.DOCS.EXAMPLE", DECISION_ID),
        ],
        "contract",
        "INV",
    );

    let judged = python_validation_errors(&[sound, two_kinds_one_prefix]);
    let mut wrong = Vec::new();

    if !judged[0].is_empty() {
        wrong.push(format!("a sound registry must pass: {:?}", judged[0]));
    }
    sole_error(
        "one symbol in two registries",
        &judged[1],
        "symbol INV.DOCS.EXAMPLE already names",
        &mut wrong,
    );

    assert!(
        wrong.is_empty(),
        "one symbol may name two records:\n{}",
        wrong.join("\n")
    );
}

/// Заменённое решение называет того, кто его заменил.
///
/// `spec/arch/README.md` описывает замену как парную правку: у нового `supersedes`, у
/// старого `status: superseded` и `superseded-by`. Статус без преемника — тупик: текст
/// старого решения не правят никогда, так что пришедший по символу узнаёт, что решение
/// отменено, и не узнаёт, чем. Обратная половина не лучше: преемник, названный при живом
/// статусе, объявляет замену, которой не было.
#[test]
fn a_superseded_decision_names_its_successor() {
    let replaced = with_prop(
        with_prop(
            plain_decision(DECISION_FILE, DECISION_ID),
            "status",
            "superseded",
        ),
        "superseded-by",
        SUCCESSOR_ID,
    );
    let successor = with_prop(
        plain_decision(SUCCESSOR_FILE, SUCCESSOR_ID),
        "supersedes",
        &format!("[{DECISION_ID}]"),
    );
    let sound = registry_case(vec![replaced, successor.clone()]);
    let status_without_successor = registry_case(vec![with_prop(
        plain_decision(DECISION_FILE, DECISION_ID),
        "status",
        "superseded",
    )]);
    // Обе половины пары написаны, не написан только статус: связь цела, а индекс
    // по-прежнему показывает заменённое решение действующим.
    let successor_without_status = registry_case(vec![
        with_prop(
            plain_decision(DECISION_FILE, DECISION_ID),
            "superseded-by",
            SUCCESSOR_ID,
        ),
        successor,
    ]);

    let judged =
        python_validation_errors(&[sound, status_without_successor, successor_without_status]);
    let mut wrong = Vec::new();

    if !judged[0].is_empty() {
        wrong.push(format!("a written supersession must pass: {:?}", judged[0]));
    }
    sole_error(
        "a superseded decision with no successor",
        &judged[1],
        "`status: superseded` names no successor",
        &mut wrong,
    );
    sole_error(
        "a successor named by a decision that still lives",
        &judged[2],
        "without `status: superseded`",
        &mut wrong,
    );

    assert!(
        wrong.is_empty(),
        "a decision may be replaced by nobody:\n{}",
        wrong.join("\n")
    );
}

/// Замену объявляют обе записи, и порознь их половины лгут по-разному.
///
/// `supersedes` читают от предка к потомку, `superseded-by` — обратно. Написанная с
/// одной стороны связь даёт две разные истории одного решения, и обе выглядят правдой:
/// по одной запись заменена, по другой — нет. Претензия здесь одна на пару, чья бы
/// половина ни молчала, иначе автор ищет вторую правку там, где её нет.
#[test]
fn a_supersession_is_recorded_by_both_decisions() {
    let replaced = with_prop(
        with_prop(
            plain_decision(DECISION_FILE, DECISION_ID),
            "status",
            "superseded",
        ),
        "superseded-by",
        SUCCESSOR_ID,
    );
    let silent_successor = registry_case(vec![
        replaced.clone(),
        plain_decision(SUCCESSOR_FILE, SUCCESSOR_ID),
    ]);
    let silent_predecessor = registry_case(vec![
        plain_decision(DECISION_FILE, DECISION_ID),
        with_prop(
            plain_decision(SUCCESSOR_FILE, SUCCESSOR_ID),
            "supersedes",
            &format!("[{DECISION_ID}]"),
        ),
    ]);
    let successor_is_nobody = registry_case(vec![with_prop(
        with_prop(
            plain_decision(DECISION_FILE, DECISION_ID),
            "status",
            "superseded",
        ),
        "superseded-by",
        "DEC.2026-09-18.NOBODY",
    )]);
    // Правило заменить нельзя: его правят вместе с решением. Названное в `supersedes`,
    // оно молча объявляло бы замену, которой реестр не знает.
    let a_rule_in_place_of_a_decision = registry_case(vec![
        with_prop(
            decision_file(SUCCESSOR_FILE, SUCCESSOR_ID, "INV.DOCS.EXAMPLE"),
            "supersedes",
            "[INV.DOCS.EXAMPLE]",
        ),
        rule_file("INV.DOCS.EXAMPLE.md", "INV.DOCS.EXAMPLE", SUCCESSOR_ID),
    ]);

    let judged = python_validation_errors(&[
        silent_successor,
        silent_predecessor,
        successor_is_nobody,
        a_rule_in_place_of_a_decision,
    ]);
    let mut wrong = Vec::new();

    sole_error(
        "a successor that does not claim its predecessor",
        &judged[0],
        &format!("does not name {DECISION_ID} in `supersedes`"),
        &mut wrong,
    );
    sole_error(
        "a predecessor that does not name its successor",
        &judged[1],
        &format!("does not name {SUCCESSOR_ID} in `superseded-by`"),
        &mut wrong,
    );
    sole_error(
        "a successor no record answers to",
        &judged[2],
        "supersession names DEC.2026-09-18.NOBODY, which is no decision",
        &mut wrong,
    );
    sole_error(
        "a rule named where a decision belongs",
        &judged[3],
        "supersession names INV.DOCS.EXAMPLE, which is no decision",
        &mut wrong,
    );

    assert!(
        wrong.is_empty(),
        "half a supersession passes for a whole one:\n{}",
        wrong.join("\n")
    );
}

/// Значение поля с закрытым перечнем входит в перечень.
///
/// Гейт читает `status` и `governs` точным равенством, поэтому значение вне перечня не
/// отвергалось само собой: оно оказывалось «ни тем ни другим», и всякая проверка, что
/// на него ветвится, молча переставала применяться. Последняя пара фикстур показывает
/// цену буквой: тот же реестр под `active` отвергается за правило поверх непринятого
/// решения, а под `activ` проходил целиком. В индексе опечатку не видно — колонка
/// печатает значение как есть и выглядит заполненной.
#[test]
fn a_closed_field_admits_only_its_published_values() {
    let unknown_status = registry_case(vec![with_prop(
        plain_decision(DECISION_FILE, DECISION_ID),
        "status",
        "activ",
    )]);
    let unknown_axis = registry_case(vec![with_prop(
        plain_decision(DECISION_FILE, DECISION_ID),
        "governs",
        "produkt",
    )]);
    let intended = with_prop(
        with_prop(
            decision_file(DECISION_FILE, DECISION_ID, "INV.DOCS.EXAMPLE"),
            "status",
            "planned",
        ),
        "realized",
        "null",
    );
    let rule_over_an_unrealized_decision = registry_case(vec![
        intended.clone(),
        rule_file("INV.DOCS.EXAMPLE.md", "INV.DOCS.EXAMPLE", DECISION_ID),
    ]);
    let same_registry_with_a_typo = registry_case(vec![
        intended,
        with_prop(
            rule_file("INV.DOCS.EXAMPLE.md", "INV.DOCS.EXAMPLE", DECISION_ID),
            "status",
            "activ",
        ),
    ]);

    let judged = python_validation_errors(&[
        unknown_status,
        unknown_axis,
        rule_over_an_unrealized_decision,
        same_registry_with_a_typo,
    ]);
    let mut wrong = Vec::new();

    sole_error(
        "a status outside the published set",
        &judged[0],
        "`status` must be one of active, planned, superseded",
        &mut wrong,
    );
    sole_error(
        "an axis outside the published set",
        &judged[1],
        "`governs` must be one of product, process",
        &mut wrong,
    );
    sole_error(
        "an active rule over a decision that is only intended",
        &judged[2],
        "active rule cites a non-active decision",
        &mut wrong,
    );
    sole_error(
        "the same registry with the status misspelt",
        &judged[3],
        "`status` must be one of active, planned, superseded",
        &mut wrong,
    );

    assert!(
        wrong.is_empty(),
        "a field may carry a value nothing publishes:\n{}",
        wrong.join("\n")
    );
}

/// Форма значения совпадает с той, которую README публикует рядом со смыслом поля.
///
/// Форма не та — и проверка не падает, а отвечает про другое. `establishes` скаляром
/// превращал обратную проверку в поиск подстроки: решение, заводящее
/// `INV.DOCS.EXAMPLE-LONGER`, удостоверяло `INV.DOCS.EXAMPLE`. `superseded-by` списком
/// называл двух преемников там, где обещан один. `id` списком ронял разбор всего реестра
/// трассировкой `unhashable type: 'list'` — вместо строки об одной записи гейт не судил
/// ни одной.
#[test]
fn a_field_has_the_shape_the_readme_publishes() {
    let sound = registry_case(vec![
        decision_file(DECISION_FILE, DECISION_ID, "INV.DOCS.EXAMPLE"),
        rule_file("INV.DOCS.EXAMPLE.md", "INV.DOCS.EXAMPLE", DECISION_ID),
    ]);
    let a_list_written_as_one_symbol = registry_case(vec![with_prop(
        plain_decision(DECISION_FILE, DECISION_ID),
        "establishes",
        "INV.DOCS.EXAMPLE",
    )]);
    let one_symbol_written_as_a_list = registry_case(vec![with_prop(
        plain_decision(DECISION_FILE, DECISION_ID),
        "superseded-by",
        "[DEC.2026-09-18.ONE, DEC.2026-09-18.OTHER]",
    )]);
    let a_scope_that_is_not_a_list = registry_case(vec![
        decision_file(DECISION_FILE, DECISION_ID, "INV.DOCS.EXAMPLE"),
        with_prop(
            rule_file("INV.DOCS.EXAMPLE.md", "INV.DOCS.EXAMPLE", DECISION_ID),
            "scope",
            "docs",
        ),
    ]);
    // Решение здесь ничего не заводит: записи без читаемого символа в `establishes` не
    // место, и назови оно её — гейт справедливо сказал бы об этом вторым сообщением,
    // а предмет фикстуры один.
    let a_symbol_written_as_a_list = registry_case(vec![
        plain_decision(DECISION_FILE, DECISION_ID),
        with_prop(
            rule_file("INV.DOCS.EXAMPLE.md", "INV.DOCS.EXAMPLE", DECISION_ID),
            "id",
            "[INV.DOCS.EXAMPLE]",
        ),
    ]);

    let judged = python_validation_errors(&[
        sound,
        a_list_written_as_one_symbol,
        one_symbol_written_as_a_list,
        a_scope_that_is_not_a_list,
        a_symbol_written_as_a_list,
    ]);
    let mut wrong = Vec::new();

    if !judged[0].is_empty() {
        wrong.push(format!("a sound registry must pass: {:?}", judged[0]));
    }
    sole_error(
        "a list written as one symbol",
        &judged[1],
        "`establishes` must be a list",
        &mut wrong,
    );
    sole_error(
        "one symbol written as a list",
        &judged[2],
        "`superseded-by` takes a single value, not a list",
        &mut wrong,
    );
    sole_error(
        "a scope that is not a list",
        &judged[3],
        "`scope` must be a list",
        &mut wrong,
    );
    sole_error(
        "a symbol written as a list",
        &judged[4],
        "`id` takes a single value, not a list",
        &mut wrong,
    );

    assert!(
        wrong.is_empty(),
        "a field may carry a shape nothing publishes:\n{}",
        wrong.join("\n")
    );
}

/// Правило, названное решением, существует и является правилом.
///
/// `establishes` и `changes` ведут от решения к правилу: одно называет выведенное из
/// решения, другое — то, чью наблюдаемую форму решение меняет. Обратный ход
/// «правило → владелец» гейт держал, прямой не проверял вовсе, и символ, за которым
/// записи нет, проходил молча: решение обещало правило, которого никто не написал.
#[test]
fn a_decision_cites_rules_that_exist() {
    let establishes_a_ghost = registry_case(vec![with_prop(
        plain_decision(DECISION_FILE, DECISION_ID),
        "establishes",
        "[INV.DOCS.GHOST]",
    )]);
    let establishes_a_decision = registry_case(vec![
        with_prop(
            plain_decision(DECISION_FILE, DECISION_ID),
            "establishes",
            &format!("[{SUCCESSOR_ID}]"),
        ),
        plain_decision(SUCCESSOR_FILE, SUCCESSOR_ID),
    ]);
    let changes_a_ghost = registry_case(vec![with_prop(
        plain_decision(DECISION_FILE, DECISION_ID),
        "changes",
        "[CTR.WIRE.GHOST]",
    )]);

    let judged =
        python_validation_errors(&[establishes_a_ghost, establishes_a_decision, changes_a_ghost]);
    let mut wrong = Vec::new();

    sole_error(
        "a rule promised and never written",
        &judged[0],
        "establishes cites missing rule INV.DOCS.GHOST",
        &mut wrong,
    );
    sole_error(
        "a decision named where a rule belongs",
        &judged[1],
        &format!("establishes cites a non-rule {SUCCESSOR_ID}"),
        &mut wrong,
    );
    sole_error(
        "a changed rule that does not exist",
        &judged[2],
        "changes cites missing rule CTR.WIRE.GHOST",
        &mut wrong,
    );

    assert!(
        wrong.is_empty(),
        "a decision may promise a rule nobody wrote:\n{}",
        wrong.join("\n")
    );
}

//! Страж нормативного реестра: схема записей и свежесть индекса.
//!
//! Реестр описан в `spec/arch/README.md`. Проверка запускает `scripts/arch/registry.py`,
//! потому что разбор записей и порождение индекса живут там же, где формат.
//!
//! Остальным проверкам нужны сами поля, и читает их [`front_matter`] — построчно, тем же
//! подмножеством YAML. Два читателя одного формата обязаны сходиться на всём: запись,
//! которую один разобрал, а второй нет, валит второго сообщением о пропущенном пропе, и
//! ненаписанным выглядит тест, а не разбор. Сходимость держит
//! [`both_gates_read_one_front_matter_form`].

use std::collections::BTreeMap;
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

/// Значение пропа: скаляр, плоский список или `null`.
///
/// Ровно то, что разбирает `parse_front_matter` в `scripts/arch/registry.py`, и не больше:
/// запись, которой нужна вложенность, переросла свой предмет.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Prop {
    Null,
    Scalar(String),
    List(Vec<String>),
}

/// Поля записи из блока между `---`.
///
/// Разбор построчный и повторяет `parse_front_matter` из `scripts/arch/registry.py`: те же
/// два вида списка — потоковый `[a, b]` и блочный `- a` со следующей строки, — те же два
/// написания пустоты и тот же отказ на строке, которую формат не описывает. Отказ
/// возвращается значением, а не пустотой: неразобранная запись обязана выглядеть
/// неразобранной, иначе страж сообщает про ненаписанный тест там, где не понял строку.
fn front_matter(text: &str) -> Result<BTreeMap<String, Prop>, String> {
    let opened = text
        .strip_prefix("---\n")
        .ok_or_else(|| "record does not open with a front-matter block".to_owned())?;
    let closed = opened
        .find("\n---\n")
        .ok_or_else(|| "record does not open with a front-matter block".to_owned())?;

    let mut props = BTreeMap::new();
    let mut block_key: Option<String> = None;
    for (index, line) in opened[..closed].lines().enumerate() {
        let number = index + 1;
        // Пустая строка и комментарий блочный список не закрывают: его закрывает
        // только следующий ключ.
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if let Some(item) = line.trim_start().strip_prefix("- ") {
            // Продолжить можно только список, открытый пустым значением, поэтому
            // одиночный `- item` — испорченная запись, а не молча усыновлённый сирота.
            let Some(Prop::List(items)) = block_key.as_ref().and_then(|key| props.get_mut(key))
            else {
                return Err(format!(
                    "front matter line {number} starts a list with no key"
                ));
            };
            items.push(item.trim().to_owned());
            continue;
        }
        block_key = None;
        let Some((key, raw)) = line.split_once(':') else {
            return Err(format!(
                "front matter line {number} is not `key: value`: {line:?}"
            ));
        };
        let (key, raw) = (key.trim().to_owned(), raw.trim());
        let value = if let Some(inner) = raw
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            Prop::List(
                inner
                    .split(',')
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .map(str::to_owned)
                    .collect(),
            )
        } else if raw.is_empty() {
            block_key = Some(key.clone());
            Prop::List(Vec::new())
        } else if raw == "null" || raw == "~" {
            Prop::Null
        } else {
            Prop::Scalar(raw.to_owned())
        };
        props.insert(key, value);
    }
    Ok(props)
}

/// Каждый адрес `путь::имя`, названный пропом.
///
/// Повторяет `evidence_names` из `scripts/arch/registry.py`: отсутствующий проп, `null` и
/// пустой список одинаково не называют ничего, а вид списка на результат не влияет.
fn evidence_names(props: &BTreeMap<String, Prop>, key: &str) -> Vec<String> {
    match props.get(key) {
        Some(Prop::Scalar(value)) => vec![value.clone()],
        Some(Prop::List(items)) => items.clone(),
        Some(Prop::Null) | None => Vec::new(),
    }
}

/// Значение пропа так, как его прочитал разбор, — для сообщения о нарушении.
fn shown(value: Option<&Prop>) -> String {
    match value {
        None => "nothing".to_owned(),
        Some(Prop::Null) => "null".to_owned(),
        Some(Prop::Scalar(value)) => value.clone(),
        Some(Prop::List(items)) => format!("[{}]", items.join(", ")),
    }
}

/// Файлы записей одного реестра, по порядку: отчёт стража не должен зависеть от того,
/// в каком порядке каталог отдал записи.
fn record_paths(base: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(base)
        .expect("registry directory is readable")
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("md"))
        .collect();
    paths.sort();
    paths
}

fn unresolved_evidence(root: &Path, dir: &str, prop: &str) -> Vec<String> {
    let mut unresolved = Vec::new();
    for path in record_paths(&root.join(dir)) {
        let text = std::fs::read_to_string(&path).expect("record is readable");
        let props = match front_matter(&text) {
            Ok(props) => props,
            Err(error) => {
                unresolved.push(format!("{}: {error}", path.display()));
                continue;
            }
        };
        if !props.contains_key(prop) {
            unresolved.push(format!("{}: no {prop} prop", path.display()));
            continue;
        }
        for item in evidence_names(&props, prop) {
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

/// Записи, на которых формат полей можно понять двояко.
///
/// Мелкие краевые случаи живут рядом с утверждением о них, а не в `tests/fixtures/`:
/// смотреть на них нужно вместе с ним.
const FRONT_MATTER_FIXTURES: &[(&str, &str)] = &[
    (
        "check as a block list",
        "---
id: INV.DOCS.EXAMPLE
status: active
check:
  - tests/arch_registry.rs::both_gates_read_one_front_matter_form
  - tests/arch_registry.rs::a_list_reads_the_same_in_both_forms
scope:
  - docs
  - ci
---

# Правило
",
    ),
    (
        "check as a flow list",
        "---
id: INV.DOCS.EXAMPLE
status: active
check: [tests/arch_registry.rs::both_gates_read_one_front_matter_form, tests/arch_registry.rs::a_list_reads_the_same_in_both_forms]
scope: [docs, ci]
---

# Правило
",
    ),
    (
        "a blank line and a comment inside a block list",
        "---
scope:
  - docs

  # области перечислены по алфавиту
  - ci
status: active
---

# Правило
",
    ),
    (
        "both spellings of nothing",
        "---
status: planned
check: null
superseded-by: ~
---

# Правило
",
    ),
    (
        "a block list with no items",
        "---
check:
scope: [docs]
---

# Правило
",
    ),
    (
        "a list item with no key above it",
        "---
id: INV.DOCS.EXAMPLE
- tests/arch_registry.rs::orphan
---

# Правило
",
    ),
    (
        "a line that is not a pair",
        "---
id: INV.DOCS.EXAMPLE
status
---

# Правило
",
    ),
    (
        "no front-matter block at all",
        "# Правило

Текст без полей.
",
    ),
];

/// Разбирает те же тексты тем же кодом, которым их читает гейт `registry.py --check`.
const FRONT_MATTER_PROBE: &str = r#"
import json, sys

sys.dont_write_bytecode = True
sys.path.insert(0, "scripts/arch")
import registry

answer = []
for text in json.load(sys.stdin):
    try:
        props, _ = registry.parse_front_matter(text)
    except ValueError as error:
        answer.append({"rejected": str(error)})
    else:
        answer.append({"props": props})
json.dump(answer, sys.stdout, ensure_ascii=False)
"#;

/// Поля так, как их читает `scripts/arch/registry.py`: `{"props": …}` либо `{"rejected": …}`
/// на каждый текст, по порядку.
fn python_front_matter(texts: &[&str]) -> Vec<serde_json::Value> {
    let mut probe = Command::new(python())
        .arg("-c")
        .arg(FRONT_MATTER_PROBE)
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
        .write_all(&serde_json::to_vec(texts).expect("fixtures serialize"))
        .expect("probe reads its input");
    let output = probe.wait_with_output().expect("probe answers");

    assert!(
        output.status.success(),
        "registry.py cannot read front matter at all:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("probe answers json")
}

/// Поля в том же виде, в каком их отдаёт разбор `registry.py`.
fn as_json(props: &BTreeMap<String, Prop>) -> serde_json::Value {
    props
        .iter()
        .map(|(key, value)| {
            let value = match value {
                Prop::Null => serde_json::Value::Null,
                Prop::Scalar(value) => serde_json::Value::String(value.clone()),
                Prop::List(items) => items
                    .iter()
                    .cloned()
                    .map(serde_json::Value::String)
                    .collect(),
            };
            (key.clone(), value)
        })
        .collect()
}

/// Формат полей один, а читают его двое: `scripts/arch/registry.py` — потому что там же
/// живёт схема и порождается индекс, и [`front_matter`] — потому что остальным проверкам
/// нужны сами поля.
///
/// Расхождение таких читателей не выглядит расхождением. Запись, которую один разобрал, а
/// второй нет, валит второго сообщением о пропе, а не о разборе, и автор ищет ненаписанный
/// тест там, где второй читатель не понял строку; в CI это к тому же падает после того,
/// как локально прошло. Поэтому сверяются и разобранные поля, и сам факт отказа.
///
/// Текст отказа не сверяется: одинаковым его держит не формат, а совпадение.
#[test]
fn both_gates_read_one_front_matter_form() {
    let texts: Vec<&str> = FRONT_MATTER_FIXTURES
        .iter()
        .map(|(_, text)| *text)
        .collect();
    let mut wrong = Vec::new();

    for ((name, text), theirs) in FRONT_MATTER_FIXTURES
        .iter()
        .zip(python_front_matter(&texts))
    {
        let ours = front_matter(text);
        let agree = match (&ours, theirs.get("props")) {
            (Ok(props), Some(theirs)) => &as_json(props) == theirs,
            (Err(_), None) => true,
            _ => false,
        };
        if !agree {
            wrong.push(format!(
                "{name}:\n  registry.py:   {theirs}\n  front_matter:  {}",
                match &ours {
                    Ok(props) => as_json(props).to_string(),
                    Err(error) => format!("rejected: {error}"),
                }
            ));
        }
    }

    assert!(
        wrong.is_empty(),
        "the two gates read the same record differently:\n{}",
        wrong.join("\n")
    );
}

/// Блочный список и потоковый — одно значение.
///
/// Блочный вид заведён под давлением длины: `check` называет до трёх адресов вида
/// `путь::имя_теста`, и в одну строку это уже даёт под триста знаков. Если он читается
/// иначе, страж говорит «правило не называет фальсификатор» ровно там, где правило его
/// называет, — и чинят такое переписыванием записи обратно в длинную строку.
#[test]
fn a_list_reads_the_same_in_both_forms() {
    let fixture = |name: &str| {
        let (_, text) = FRONT_MATTER_FIXTURES
            .iter()
            .find(|(fixture, _)| *fixture == name)
            .expect("fixture is named");
        front_matter(text).expect("fixture parses")
    };

    let block = fixture("check as a block list");
    assert_eq!(
        block,
        fixture("check as a flow list"),
        "a list must not depend on how it is spelled"
    );
    assert_eq!(
        evidence_names(&block, "check"),
        [
            "tests/arch_registry.rs::both_gates_read_one_front_matter_form",
            "tests/arch_registry.rs::a_list_reads_the_same_in_both_forms"
        ],
        "a block list names its falsifiers; reading it as nothing is the false negative"
    );
}

/// Правило со `status: planned` обязано объявлять отсутствие проверки полем
/// `check: null`, а действующее — называть её. Пустое поле у действующего правила
/// и названная проверка у запланированного одинаково прячут состояние долга.
#[test]
fn planned_rules_declare_a_missing_check() {
    let root = repo_root();
    let mut wrong = Vec::new();

    for dir in ["spec/arch/invariants", "spec/arch/contracts"] {
        for path in record_paths(&root.join(dir)) {
            let text = std::fs::read_to_string(&path).expect("record is readable");
            let props = match front_matter(&text) {
                Ok(props) => props,
                Err(error) => {
                    wrong.push(format!("{}: {error}", path.display()));
                    continue;
                }
            };
            let status = match props.get("status") {
                Some(Prop::Scalar(value)) => value.as_str(),
                _ => "",
            };
            let check = props.get("check");

            match status {
                "planned" if check != Some(&Prop::Null) => wrong.push(format!(
                    "{}: planned rule must declare `check: null`, found {}",
                    path.display(),
                    shown(check)
                )),
                "active" if evidence_names(&props, "check").is_empty() => wrong.push(format!(
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
    // Решение держится теми же свидетельствами и тем же пропом-списком, что и правило,
    // — разбор у них общий, и расходиться им не на чем.
    let unresolved = unresolved_evidence(&root, "spec/arch/decisions", "realized");

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

    for path in record_paths(&contracts) {
        let text = std::fs::read_to_string(&path).expect("record is readable");
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_owned();

        let props = match front_matter(&text) {
            Ok(props) => props,
            Err(error) => {
                wrong.push(format!("{name}: {error}"));
                continue;
            }
        };
        let Some(Prop::Scalar(artifact)) = props.get("artifact").cloned() else {
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

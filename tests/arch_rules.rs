//! Гейт правил продукта: `spec/arch/rules/`.
//!
//! Проверяется не форма прозы, а то, без чего запись перестаёт быть обязательством:
//! у неё есть имя, это имя одно на весь реестр, названные проверки существуют, а набор
//! полей закрыт. Всё остальное — текст правила — держит разбор, а не гейт.
//!
//! Обход **рекурсивный**: записи лежат по областям продукта, и плоское чтение каталога
//! нашло бы ноль записей и прошло зелёным. Поэтому здесь же стоит пол по числу записей:
//! страж, который нечего проверять, — не страж.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Ниже этого числа реестр правил не опускался ни разу с переноса. Пол ловит не убыль
/// обязательств, а поломку обхода: неверный корень или нерекурсивное чтение дают ноль.
const AT_LEAST: usize = 120;

/// Поля записи закрыты. `id` и `check` обязательны; `gap` называет задачу, пока
/// обязательство не выполнено; `version` и `artifact` есть у записи, закрепляющей форму
/// данных. Ничего сверх — иначе прежние `status`, `governs` и `decision` вернутся по
/// одному за раз.
const ALLOWED: &[&str] = &["id", "check", "gap", "version", "artifact"];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn rules_root() -> PathBuf {
    repo_root().join("spec/arch/rules")
}

/// Все записи правил, в устойчивом порядке. README областей записью не является.
fn rule_paths(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let entries = std::fs::read_dir(dir).expect("rules directory is readable");
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(rule_paths(&path));
        } else if path.extension().is_some_and(|value| value == "md")
            && path.file_name().is_some_and(|value| value != "README.md")
        {
            found.push(path);
        }
    }
    found.sort();
    found
}

/// Поля записи: скаляр или плоский список. Больше записи не нужно, и большего здесь
/// намеренно не читают.
fn front_matter(text: &str) -> Result<BTreeMap<String, Vec<String>>, String> {
    let rest = text
        .strip_prefix("---\n")
        .ok_or_else(|| "no front matter".to_owned())?;
    let end = rest
        .find("\n---\n")
        .ok_or_else(|| "front matter is not closed".to_owned())?;
    let mut props: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut key = String::new();
    for line in rest[..end].lines() {
        if let Some(item) = line.strip_prefix("  - ") {
            if key.is_empty() {
                return Err(format!("list item before any key: {line}"));
            }
            props
                .entry(key.clone())
                .or_default()
                .push(item.trim().to_owned());
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(format!("line is neither a key nor a list item: {line}"));
        };
        key = name.trim().to_owned();
        let value = value.trim();
        let values = if value.is_empty() {
            Vec::new()
        } else if let Some(inner) = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
            inner
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_owned)
                .collect()
        } else {
            vec![value.to_owned()]
        };
        props.insert(key.clone(), values);
    }
    Ok(props)
}

fn read_rules() -> Vec<(String, BTreeMap<String, Vec<String>>)> {
    rule_paths(&rules_root())
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).expect("rule is readable");
            let shown = path
                .strip_prefix(rules_root())
                .unwrap_or(&path)
                .display()
                .to_string();
            let props = front_matter(&text).unwrap_or_else(|error| panic!("{shown}: {error}"));
            (shown, props)
        })
        .collect()
}

#[test]
fn the_registry_is_read_whole() {
    let rules = read_rules();
    assert!(
        rules.len() >= AT_LEAST,
        "прочитано {} записей при поле {AT_LEAST}: корень или обход сломаны",
        rules.len()
    );
}

/// Имя записи — единственный способ на неё сослаться, и оно обязано быть одно.
///
/// Прежде это держала пара «символ и путь пишут друг друга»; имя файла теперь называет
/// тему, а не символ, поэтому уникальность проверяется прямо.
#[test]
fn a_name_belongs_to_exactly_one_rule() {
    let mut seen: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (shown, props) in read_rules() {
        let Some(id) = props.get("id").and_then(|values| values.first()) else {
            panic!("{shown}: запись без поля `id`");
        };
        seen.entry(id.clone()).or_default().push(shown);
    }
    let clashes: Vec<String> = seen
        .iter()
        .filter(|(_, files)| files.len() > 1)
        .map(|(id, files)| format!("{id}: {}", files.join(", ")))
        .collect();

    assert!(
        clashes.is_empty(),
        "одно имя у нескольких записей:\n{}",
        clashes.join("\n")
    );
}

/// Набор полей закрыт: иначе прежняя схема возвращается по полю за раз.
#[test]
fn a_rule_carries_only_the_fields_the_readme_publishes() {
    let mut wrong = Vec::new();
    for (shown, props) in read_rules() {
        for key in props.keys() {
            if !ALLOWED.contains(&key.as_str()) {
                wrong.push(format!("{shown}: поле `{key}` схемой не предусмотрено"));
            }
        }
        if !props.contains_key("id") {
            wrong.push(format!("{shown}: нет поля `id`"));
        }
        if !props.contains_key("check") {
            wrong.push(format!("{shown}: нет поля `check`"));
        }
    }

    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Названная проверка существует. Гейт не судит, что она доказывает, — он не даёт
/// сослаться на то, чего нет.
#[test]
fn every_named_check_exists() {
    let root = repo_root();
    let mut unresolved = Vec::new();
    for (shown, props) in read_rules() {
        for address in props.get("check").into_iter().flatten() {
            let Some((file, name)) = address.split_once("::") else {
                unresolved.push(format!("{shown}: адрес без имени проверки: {address}"));
                continue;
            };
            let source = root.join(file);
            let Ok(text) = std::fs::read_to_string(&source) else {
                unresolved.push(format!("{shown}: нет файла {file}"));
                continue;
            };
            if !text.contains(&format!("fn {name}(")) {
                unresolved.push(format!("{shown}: в {file} нет проверки {name}"));
            }
        }
    }

    assert!(unresolved.is_empty(), "{}", unresolved.join("\n"));
}

/// Запись без проверок называет задачу: обязательство без свидетельства и без разрыва —
/// обещание, за которым никто не стоит.
#[test]
fn a_rule_without_checks_names_its_gap() {
    let mut wrong = Vec::new();
    for (shown, props) in read_rules() {
        let empty = props.get("check").is_none_or(|values| values.is_empty());
        let gap = props.get("gap").and_then(|values| values.first());
        match (empty, gap) {
            (true, None) => wrong.push(format!("{shown}: нет ни проверок, ни `gap`")),
            (false, Some(_)) => {
                wrong.push(format!("{shown}: `gap` стоит при заполненном `check`"));
            }
            _ => {}
        }
        if let Some(gap) = gap {
            if !gap.starts_with("https://github.com/") {
                wrong.push(format!("{shown}: `gap` не ведёт на задачу: {gap}"));
            }
        }
    }

    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Запись, закрепляющая форму данных, называет свой артефакт и номер формы.
///
/// Номер растёт вместе с составом полей; что он вырос, гейт не доказывает — это разбор.
/// Здесь держится меньшее и проверяемое: артефакт лежит на диске, номер — целое от единицы.
#[test]
fn a_pinned_form_names_an_artifact_that_exists() {
    let root = repo_root();
    let mut wrong = Vec::new();
    for (shown, props) in read_rules() {
        let artifact = props.get("artifact").and_then(|values| values.first());
        let version = props.get("version").and_then(|values| values.first());
        match (artifact, version) {
            (None, None) => continue,
            (Some(artifact), Some(version)) => {
                if !root.join(artifact).is_file() {
                    wrong.push(format!("{shown}: артефакта нет на диске: {artifact}"));
                }
                if version.parse::<u32>().unwrap_or(0) == 0 {
                    wrong.push(format!(
                        "{shown}: номер формы не целое от единицы: {version}"
                    ));
                }
            }
            (Some(_), None) => wrong.push(format!("{shown}: артефакт без номера формы")),
            (None, Some(_)) => wrong.push(format!("{shown}: номер формы без артефакта")),
        }
    }

    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

//! Сверка живого ответа с формой `data`, объявленной для его команды.
//!
//! Схемы лежат в `docs/schemas/command-data/`, индекс называет формы каждой команды.
//! Проверка живёт в общей обвязке, а не в одном наборе: живой прогон каждой команды
//! стоит там, где для неё уже есть окружение, — а форма у всех одна и та же.
#![allow(dead_code)]

use std::fmt::Debug;
use std::fs;
use std::path::PathBuf;

use serde_json::Value;

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub fn form_index() -> Value {
    let path = repo_root().join("docs/schemas/command-data/index.json");
    let text = fs::read_to_string(&path).expect("index artefact is present");
    serde_json::from_str(&text).expect("index is valid json")
}

pub fn form_schema(slug: &str) -> Value {
    let path = repo_root().join(format!("docs/schemas/command-data/{slug}.schema.json"));
    let text = fs::read_to_string(&path).expect("form artefact is present");
    serde_json::from_str(&text).expect("form is valid json")
}

pub fn slug_list(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|slugs| {
            slugs
                .iter()
                .map(|slug| slug.as_str().expect("slug is a string").to_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// Сверяет `data` с формами самой команды, без общих форм отказа.
///
/// Команда может отвечать несколькими формами (`extensions` читает состав одной, меняет
/// другой), поэтому годится любая из её форм, но хотя бы одна обязана подойти. Общие формы
/// отказа сюда не входят: иначе ответ без предмета команды прошёл бы проверку, ничего не
/// сказав о её форме.
pub fn assert_data_matches_its_command_form(payload: &Value, context: &str) {
    let command = payload["command"]
        .as_str()
        .unwrap_or_else(|| panic!("{context}: the reply names no command: {payload}"));
    assert_data_matches_one_of(
        &payload["data"],
        &format!("{context} (`{command}`)"),
        &declared_forms(command),
    );
}

/// Сверяет `data` с названными формами: хотя бы одна обязана подойти. Единственное место,
/// где тесты читают формы `data` и сверяют с ними ответ.
pub fn assert_data_matches_one_of<S: AsRef<str> + Debug>(data: &Value, context: &str, slugs: &[S]) {
    assert!(!slugs.is_empty(), "{context}: no data form is declared");
    let mut failures = Vec::new();
    for slug in slugs {
        let slug = slug.as_ref();
        let schema = form_schema(slug);
        let validator = jsonschema::validator_for(&schema).expect("form compiles");
        let errors: Vec<String> = validator
            .iter_errors(data)
            .map(|error| format!("{} at {}", error, error.instance_path))
            .collect();
        if errors.is_empty() {
            return;
        }
        failures.push(format!("{slug}:\n{}", errors.join("\n")));
    }
    panic!("{context}: data matches none of the forms {slugs:?}\n{failures:#?}");
}

/// Формы, объявленные для команды, без общих.
pub fn declared_forms(command: &str) -> Vec<String> {
    slug_list(&form_index()["forms"][command])
}

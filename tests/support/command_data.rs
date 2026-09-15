//! Сверка живого ответа с формой `data`, объявленной для его команды.
//!
//! Схемы лежат в `docs/schemas/command-data/`, индекс называет формы каждой команды.
//! Проверка живёт в общей обвязке, а не в одном наборе: живой прогон каждой команды
//! стоит там, где для неё уже есть окружение, — а форма у всех одна и та же.
#![allow(dead_code)]

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

/// Сверяет `data` с формой, объявленной для названного командой имени.
///
/// Команда может отвечать несколькими формами (`extensions` читает состав одной, меняет
/// другой), а отказ до диспетчеризации печатает общую для всех команд, — поэтому годится
/// любая из объявленных, но хотя бы одна обязана подойти.
pub fn assert_data_matches_a_declared_form(payload: &Value, context: &str) {
    let command = payload["command"]
        .as_str()
        .unwrap_or_else(|| panic!("{context}: the reply names no command: {payload}"));
    let index = form_index();
    let mut slugs = slug_list(&index["forms"][command]);
    slugs.extend(slug_list(&index["shared"]));
    assert!(
        !slugs.is_empty(),
        "{context}: command `{command}` declares no data form"
    );

    let data = &payload["data"];
    let mut failures = Vec::new();
    for slug in &slugs {
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
    panic!("{context}: data matches none of the forms declared for `{command}`\n{failures:#?}");
}

/// Формы, объявленные для команды, без общих.
pub fn declared_forms(command: &str) -> Vec<String> {
    slug_list(&form_index()["forms"][command])
}

//! Форма поля `data` у каждой команды.
//!
//! Конверт (`docs/schemas/command-envelope.schema.json`) закрепляет оболочку ответа и
//! оставляет `data` командой необъявленным. Предмет команды живёт именно там, поэтому
//! без отдельной формы самая большая часть ответа не удерживается ничем: поле можно
//! переименовать, сделать необязательным или убрать, и ни одна проверка не упадёт.
//!
//! Здесь перечислены все формы `data`, которые раннер печатает. Каждая порождается из
//! типа, который её сериализует, и лежит рядом файлом-схемой: расхождение между типом и
//! файлом валит проверку свежести, а расхождение между файлом и живым ответом — проверку
//! `tests/contract_command_data.rs`.

// Формы читает проверка свежести и генератор артефактов; в продуктовом пути таблицу
// никто не зовёт — её работа сделана до сборки, файлами.
#![allow(dead_code)]

use schemars::schema_for;
use serde_json::Value;

const REPOSITORY_RAW_SCHEMA_BASE: &str =
    "https://raw.githubusercontent.com/IngvarConsulting/v8-runner-rust/master/docs/schemas/command-data";

/// Каталог, в котором лежат порождённые формы.
pub const COMMAND_DATA_SCHEMA_DIR: &str = "docs/schemas/command-data";

/// Перечень форм: какая команда какой формой отвечает.
///
/// Потребитель ответа видит только поле `command`, поэтому соответствие «команда —
/// форма» само по себе часть обещания и лежит файлом рядом со схемами.
pub const COMMAND_DATA_INDEX_PATH: &str = "docs/schemas/command-data/index.json";

/// Одна опубликованная форма `data`.
pub struct CommandDataForm {
    /// Значение поля `command` в конверте, который несёт эту форму.
    pub command: &'static str,
    /// Имя файла без расширения; оно же — хвост символа контракта.
    pub slug: &'static str,
    /// Порождённая схема.
    pub schema: Value,
}

impl CommandDataForm {
    /// Путь артефакта относительно корня репозитория.
    pub fn artifact_path(&self) -> String {
        format!("{COMMAND_DATA_SCHEMA_DIR}/{}.schema.json", self.slug)
    }
}

/// Команда, под которой объявлены формы, общие для всех команд.
///
/// Отказ до диспетчеризации печатает одну и ту же форму, какую бы команду ни просили,
/// поэтому привязывать её к именам по одной — значит повторить её восемнадцать раз.
pub const SHARED_FORM_COMMAND: &str = "*";

macro_rules! command_data_forms {
    ($($command:literal, $slug:literal => $ty:ty ;)*) => {
        /// Все формы `data`, по одной на команду ответа.
        pub fn command_data_forms() -> Vec<CommandDataForm> {
            vec![$(
                CommandDataForm {
                    command: $command,
                    slug: $slug,
                    schema: generated_schema(schema_for!($ty), $slug),
                },
            )*]
        }
    };
}

command_data_forms! {
    "version", "version" => crate::app::VersionInfo;
    "bootstrap", "bootstrap" => crate::domain::bootstrap::BootstrapResult;
    "config init", "config-init" => crate::domain::config_init::ConfigInitResult;
    "tools download", "tools-download" => crate::domain::tools_download::ToolsDownloadResult;
    "init", "init" => crate::domain::init::InitResult;
    "extensions", "extensions" => crate::domain::extensions::ExtensionsResult;
    "extensions", "extensions-inventory" => crate::domain::extensions::ExtensionInventoryResult;
    "build", "build" => crate::domain::build::BuildResult;
    "load", "load" => crate::cli::execute::LoadJsonData<'static>;
    "test", "test" => crate::command_envelope::TestEnvelopeData;
    "dump", "dump" => crate::domain::dump::DumpResult;
    "infobase.configuration.export", "infobase-configuration-export"
        => crate::domain::infobase_export::ExportConfigurationPackageResult;
    "infobase.dump", "infobase-dump"
        => crate::domain::infobase_export::ExportInfobaseSnapshotResult;
    "infobase.restore", "infobase-restore"
        => crate::domain::infobase_export::RestoreInfobaseSnapshotResult;
    "convert", "convert" => crate::domain::convert::ConvertResult;
    "make", "make" => crate::cli::execute::ArtifactsJsonData<'static>;
    "syntax", "syntax" => crate::domain::syntax::SyntaxCheckResult;
    "launch", "launch" => crate::domain::launch::LaunchResult;
    "*", "refusal" => crate::cli::output::RefusalData;
    "*", "mcp-refusal" => crate::mcp::service::McpRefusalData;
}

/// Соответствие «команда — формы её ответа» в том же порядке, в каком объявлены формы.
pub fn command_data_index() -> Value {
    let mut entries = serde_json::Map::new();
    let mut shared = Vec::new();
    for form in command_data_forms() {
        if form.command == SHARED_FORM_COMMAND {
            shared.push(Value::String(form.slug.to_owned()));
            continue;
        }
        entries
            .entry(form.command.to_owned())
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .expect("array of slugs")
            .push(Value::String(form.slug.to_owned()));
    }
    serde_json::json!({
        "_comment": "Порождается UPDATE_COMMAND_DATA_SCHEMAS=1 cargo test generated_command_data_schemas_are_current; руками не правится.",
        "forms": Value::Object(entries),
        "shared": Value::Array(shared),
    })
}

fn generated_schema(schema: schemars::Schema, slug: &str) -> Value {
    let mut value = serde_json::to_value(schema).expect("schema json");
    close_every_object(&mut value);
    let object = value.as_object_mut().expect("root schema object");
    object.insert(
        "$id".to_owned(),
        Value::String(format!("{REPOSITORY_RAW_SCHEMA_BASE}/{slug}.schema.json")),
    );
    value
}

/// Закрывает список полей у каждого объекта формы.
///
/// Без этого проверка против схемы пропускает любое добавленное поле, и форма ловит
/// только удаление и переименование. Обещание раннера — весь состав ответа, поэтому
/// новое поле обязано менять версию формы, а не появляться молча.
///
/// Открытыми остаются три случая, где закрытый список запретил бы законное поле:
/// словарь с произвольными ключами (у него нет `properties`), объект, чьи поля приходят
/// из соседнего `$ref` или ветки `oneOf`, и определение, на которое такой объект
/// ссылается: тег варианта лежит в ссылающемся объекте, а не в определении.
fn close_every_object(value: &mut Value) {
    let open = definitions_that_carry_a_tag_elsewhere(value);
    close_objects(value, &open, None);
}

const BRINGS_FIELDS_FROM_ELSEWHERE: [&str; 4] = ["$ref", "allOf", "anyOf", "oneOf"];

/// Имена `$defs`, к которым ссылающийся объект добавляет свои поля.
fn definitions_that_carry_a_tag_elsewhere(value: &Value) -> std::collections::BTreeSet<String> {
    let mut found = std::collections::BTreeSet::new();
    collect_tagged_definitions(value, &mut found);
    found
}

fn collect_tagged_definitions(value: &Value, found: &mut std::collections::BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            if object.contains_key("properties") {
                if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
                    if let Some(name) = reference.strip_prefix("#/$defs/") {
                        found.insert(name.to_owned());
                    }
                }
            }
            for nested in object.values() {
                collect_tagged_definitions(nested, found);
            }
        }
        Value::Array(items) => {
            for nested in items {
                collect_tagged_definitions(nested, found);
            }
        }
        _ => {}
    }
}

fn close_objects(
    value: &mut Value,
    open: &std::collections::BTreeSet<String>,
    definition: Option<&str>,
) {
    match value {
        Value::Object(object) => {
            let stays_open = definition.is_some_and(|name| open.contains(name));
            if object.contains_key("properties")
                && !object.contains_key("additionalProperties")
                && !stays_open
                && !BRINGS_FIELDS_FROM_ELSEWHERE
                    .iter()
                    .any(|keyword| object.contains_key(*keyword))
            {
                object.insert("additionalProperties".to_owned(), Value::Bool(false));
            }
            let inside_defs = definition.is_none() && object.contains_key("$defs");
            for (key, nested) in object.iter_mut() {
                let named = if inside_defs && key == "$defs" {
                    None
                } else {
                    definition
                };
                if inside_defs && key == "$defs" {
                    if let Value::Object(defs) = nested {
                        for (name, schema) in defs.iter_mut() {
                            close_objects(schema, open, Some(name));
                        }
                        continue;
                    }
                }
                close_objects(nested, open, named);
            }
        }
        Value::Array(items) => {
            for nested in items {
                close_objects(nested, open, definition);
            }
        }
        _ => {}
    }
}

/// Тот же вид, что у схем конфигурации: отсортированные ключи и перевод строки в конце.
pub fn schema_json_pretty(schema: &Value) -> String {
    let mut text = serde_json::to_string_pretty(schema).expect("schema json");
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::Path;

    /// Форма порождается из типа и лежит файлом. Пока файл совпадает с порождённым,
    /// переименованное или исчезнувшее поле видно в диффе, а не у потребителя.
    #[test]
    fn generated_command_data_schemas_are_current() {
        maybe_update_artifacts();
        for form in command_data_forms() {
            let path = form.artifact_path();
            let actual = std::fs::read_to_string(&path)
                .unwrap_or_else(|_| panic!("artifact {path} is present"));
            assert_eq!(
                actual,
                schema_json_pretty(&form.schema),
                "{path} is stale; rerun UPDATE_COMMAND_DATA_SCHEMAS=1 cargo test generated_command_data_schemas_are_current"
            );
        }

        let index = std::fs::read_to_string(COMMAND_DATA_INDEX_PATH).expect("index artifact");
        assert_eq!(
            index,
            schema_json_pretty(&command_data_index()),
            "{COMMAND_DATA_INDEX_PATH} is stale; rerun UPDATE_COMMAND_DATA_SCHEMAS=1 cargo test generated_command_data_schemas_are_current"
        );
    }

    /// Ни одна форма не делит файл с другой: иначе версия одной команды молча
    /// перезаписывала бы обещание другой.
    #[test]
    fn every_command_data_form_owns_its_own_file() {
        let mut slugs = BTreeSet::new();
        for form in command_data_forms() {
            assert!(
                slugs.insert(form.slug),
                "slug {} is used by more than one form",
                form.slug
            );
        }
    }

    fn maybe_update_artifacts() {
        if std::env::var_os("UPDATE_COMMAND_DATA_SCHEMAS").is_none() {
            return;
        }
        std::fs::create_dir_all(COMMAND_DATA_SCHEMA_DIR).expect("schema dir");
        for form in command_data_forms() {
            let path = form.artifact_path();
            std::fs::write(Path::new(&path), schema_json_pretty(&form.schema))
                .expect("write schema artifact");
        }
        std::fs::write(
            Path::new(COMMAND_DATA_INDEX_PATH),
            schema_json_pretty(&command_data_index()),
        )
        .expect("write index artifact");
    }
}

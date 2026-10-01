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

/// Основание адреса схем, лежащих рядом с формами команд.
pub(crate) const REPOSITORY_RAW_SCHEMA_ROOT: &str =
    "https://raw.githubusercontent.com/IngvarConsulting/v8-runner-rust/master/docs/schemas";

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
    "clone", "clone" => crate::domain::bootstrap::BootstrapResult;
    "init", "init" => crate::domain::config_init::ConfigInitResult;
    "tools download", "tools-download" => crate::domain::tools_download::ToolsDownloadResult;
    "infobase create", "infobase-create" => crate::domain::init::InitResult;
    "extensions", "extensions" => crate::domain::extensions::ExtensionsResult;
    "extensions", "extensions-inventory" => crate::domain::extensions::ExtensionInventoryResult;
    "push", "push" => crate::domain::build::BuildResult;
    "upload", "upload" => crate::cli::execute::LoadJsonData<'static>;
    "test", "test" => crate::command_envelope::TestEnvelopeData;
    "pull", "pull" => crate::domain::dump::DumpResult;
    "download", "download"
        => crate::domain::infobase_export::ExportConfigurationPackageResult;
    "infobase.dump", "infobase-dump"
        => crate::domain::infobase_export::ExportInfobaseSnapshotResult;
    "infobase.restore", "infobase-restore"
        => crate::domain::infobase_export::RestoreInfobaseSnapshotResult;
    "convert", "convert" => crate::domain::convert::ConvertResult;
    "make", "make" => crate::cli::execute::ArtifactsJsonData<'static>;
    "check", "check" => crate::domain::syntax::SyntaxCheckResult;
    "launch", "launch" => crate::domain::launch::LaunchResult;
    "publish", "publish" => crate::domain::publish::PublishResult;
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

pub(crate) fn generated_schema(schema: schemars::Schema, slug: &str) -> Value {
    let mut value = serde_json::to_value(schema).expect("schema json");
    inline_tagged_variants(&mut value);
    close_every_object(&mut value);
    let object = value.as_object_mut().expect("root schema object");
    object.insert(
        "$id".to_owned(),
        Value::String(format!("{REPOSITORY_RAW_SCHEMA_BASE}/{slug}.schema.json")),
    );
    value
}

/// Вставляет тело варианта размеченного перечисления в ветку, которая несёт его тег.
///
/// `schemars` раскладывает такое перечисление на ветку `oneOf` с тегом и `$ref` на
/// определение варианта: тег лежит в ссылающемся объекте, тело — в `$defs`. Закрыть
/// нельзя ни то, ни другое — каждый запретил бы поля соседа, и состав полей варианта
/// оставался открытым. После вставки ветка описывает вариант целиком и закрывается
/// обычным порядком; определения, на которые больше никто не ссылается, убираются.
fn inline_tagged_variants(value: &mut Value) {
    let definitions = value
        .get("$defs")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if definitions.is_empty() {
        return;
    }
    inline_into_branches(value, &definitions);
    drop_unreferenced_definitions(value);
}

fn inline_into_branches(value: &mut Value, definitions: &serde_json::Map<String, Value>) {
    match value {
        Value::Object(object) => {
            let target = object
                .get("$ref")
                .and_then(Value::as_str)
                .filter(|_| object.contains_key("properties"))
                .and_then(|reference| reference.strip_prefix("#/$defs/"))
                .map(str::to_owned);
            if let Some(name) = target {
                if let Some(Value::Object(body)) = definitions.get(&name) {
                    object.remove("$ref");
                    merge_variant_body(object, body, &name);
                }
            }
            for nested in object.values_mut() {
                inline_into_branches(nested, definitions);
            }
        }
        Value::Array(items) => {
            for nested in items {
                inline_into_branches(nested, definitions);
            }
        }
        _ => {}
    }
}

/// Поля и обязательность тела добавляются к тегу. Совпадение имени тега с полем тела —
/// поломка формы, а не случай для тихого выбора победителя.
fn merge_variant_body(
    branch: &mut serde_json::Map<String, Value>,
    body: &serde_json::Map<String, Value>,
    name: &str,
) {
    if let Some(Value::Object(fields)) = body.get("properties") {
        let own = branch
            .entry("properties".to_owned())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        let own = own.as_object_mut().expect("properties is an object");
        for (field, schema) in fields {
            assert!(
                !own.contains_key(field),
                "variant `{name}` names `{field}` both as its tag and as its field"
            );
            own.insert(field.clone(), schema.clone());
        }
    }
    if let Some(Value::Array(required)) = body.get("required") {
        let own = branch
            .entry("required".to_owned())
            .or_insert_with(|| Value::Array(Vec::new()));
        let own = own.as_array_mut().expect("required is an array");
        for field in required {
            if !own.contains(field) {
                own.push(field.clone());
            }
        }
    }
    for (keyword, schema) in body {
        if keyword == "properties" || keyword == "required" {
            continue;
        }
        branch
            .entry(keyword.clone())
            .or_insert_with(|| schema.clone());
    }
}

/// Определения, на которые после вставки никто не ссылается, из формы убираются.
///
/// Живым считается то, на что ссылаются вне `$defs`, и всё, до чего можно дойти по
/// ссылкам оттуда: пара определений, ссылающихся друг на друга, сама себя живой не
/// делает. Поэтому обход — неподвижная точка, а не один проход.
fn drop_unreferenced_definitions(value: &mut Value) {
    let Some(definitions) = value.get("$defs").and_then(Value::as_object).cloned() else {
        return;
    };
    let mut live = std::collections::BTreeSet::new();
    let mut outside = value.clone();
    if let Some(object) = outside.as_object_mut() {
        object.remove("$defs");
    }
    collect_references(&outside, &mut live);
    loop {
        let mut grown = live.clone();
        for name in &live {
            if let Some(schema) = definitions.get(name) {
                collect_references(schema, &mut grown);
            }
        }
        if grown == live {
            break;
        }
        live = grown;
    }
    if let Some(defs) = value.get_mut("$defs").and_then(Value::as_object_mut) {
        defs.retain(|name, _| live.contains(name));
        if defs.is_empty() {
            value
                .as_object_mut()
                .expect("root schema object")
                .remove("$defs");
        }
    }
}

fn collect_references(value: &Value, found: &mut std::collections::BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            if let Some(name) = object
                .get("$ref")
                .and_then(Value::as_str)
                .and_then(|reference| reference.strip_prefix("#/$defs/"))
            {
                found.insert(name.to_owned());
            }
            for nested in object.values() {
                collect_references(nested, found);
            }
        }
        Value::Array(items) => {
            for nested in items {
                collect_references(nested, found);
            }
        }
        _ => {}
    }
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

    /// Состав полей закрыт у каждого объекта каждой формы, включая варианты
    /// размеченного перечисления: их тело вставляется в ветку с тегом, поэтому
    /// закрывается обычным порядком. Без вставки такие объекты оставались открытыми,
    /// и добавленное внутрь варианта поле проходило молча.
    #[test]
    fn every_object_of_every_form_closes_its_field_list() {
        fn open_objects(value: &Value, path: &str, found: &mut Vec<String>) {
            match value {
                Value::Object(object) => {
                    if object.contains_key("properties")
                        && !object.contains_key("additionalProperties")
                    {
                        found.push(path.to_owned());
                    }
                    for (key, nested) in object {
                        open_objects(nested, &format!("{path}/{key}"), found);
                    }
                }
                Value::Array(items) => {
                    for (index, nested) in items.iter().enumerate() {
                        open_objects(nested, &format!("{path}[{index}]"), found);
                    }
                }
                _ => {}
            }
        }

        for form in command_data_forms() {
            let mut found = Vec::new();
            open_objects(&form.schema, "", &mut found);
            assert!(
                found.is_empty(),
                "form `{}` leaves objects open: {found:?}",
                form.slug
            );
        }
    }

    /// Фальсификатор для предыдущего: поле, добавленное внутрь варианта, форму валит.
    /// До вставки тела варианта в ветку такой документ проходил проверку.
    #[test]
    fn a_field_added_inside_a_variant_breaks_the_form() {
        let form = command_data_forms()
            .into_iter()
            .find(|form| form.slug == "check")
            .expect("check form");
        let validator = jsonschema::validator_for(&form.schema).expect("form compiles");
        let mut issue = serde_json::json!({
            "kind": "module",
            "path": "src/cf/CommonModules/Демо/Ext/Module.bsl",
            "line": 42,
            "column": 5,
            "severity": "ERROR",
            "message": "Переменная не определена"
        });
        let document = |issue: &Value| {
            serde_json::json!({
                "provider": {"selected": "designer", "origin": {"kind": "default"}},
                "provider_dispatched": true,
                "status": "issues_found",
                "exit_code": 1,
                "check_name": "designer-config",
                "issues": [issue],
                "summary": {"errors": 1, "warnings": 0, "info": 0},
                "duration_ms": 321
            })
        };
        assert!(
            validator.is_valid(&document(&issue)),
            "the declared shape of a variant is accepted"
        );
        issue["invented"] = Value::String("field".to_owned());
        assert!(
            !validator.is_valid(&document(&issue)),
            "a field invented inside a variant must break the form"
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

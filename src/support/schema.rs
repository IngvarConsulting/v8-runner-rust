//! Чтение порождённых JSON-схем в тестах.

/// Все строки `const` и `enum` под данным узлом схемы — значения закрытого перечисления так,
/// как их пишет схема. `schemars` раскладывает перечисление на ветку со списком `enum` и по
/// ветке `const` на каждый вариант с пояснением, поэтому значения собираются из обеих форм.
pub(crate) fn enum_values(schema: &serde_json::Value) -> Vec<String> {
    let mut found = Vec::new();
    collect_enum_values(schema, &mut found);
    found
}

fn collect_enum_values(value: &serde_json::Value, found: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(object) => {
            if let Some(serde_json::Value::String(name)) = object.get("const") {
                found.push(name.clone());
            }
            if let Some(serde_json::Value::Array(members)) = object.get("enum") {
                found.extend(
                    members
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(ToOwned::to_owned),
                );
            }
            for nested in object.values() {
                collect_enum_values(nested, found);
            }
        }
        serde_json::Value::Array(items) => {
            for nested in items {
                collect_enum_values(nested, found);
            }
        }
        _ => {}
    }
}

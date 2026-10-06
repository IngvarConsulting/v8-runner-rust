//! Каждый ключ, который модель конфигурации принимает как синоним, есть в опубликованной
//! схеме с `deprecated: true` (`INV.CONFIG.A-KEY-SYNONYM-IS-MARKED-DEPRECATED-IN-THE-SCHEMA`).
//!
//! Синонимы не перечисляются здесь заново: тест читает их из модели — атрибуты
//! `#[serde(alias = "…")]` у типов, достижимых из `AppConfig`, — и ищет каждый в схеме.
//! Новый синоним в модели без пометки в схеме роняет тест под своим именем.

mod guardrail_support;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

use guardrail_support::{collect_rust_files, production_items};
use serde_json::Value;
use syn::{Attribute, Fields, Item, LitStr, Type};

/// Корень модели: то, во что загрузчик читает `v8project.yaml` со слоем машины.
const MODEL_ROOT: &str = "AppConfig";
/// Каталоги, где живут типы модели.
const MODEL_LAYERS: &[&str] = &["src/config", "src/domain"];
/// Опубликованные схемы конфигурации.
const SCHEMAS: &[&str] = &[
    "docs/schemas/v8project.schema.json",
    "docs/schemas/v8project.local.schema.json",
];

/// Синоним модели: где объявлен, каким именем принимается и чему равен.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ModelSynonym {
    owner: String,
    canonical: String,
    alias: String,
}

/// Поле или вариант: атрибуты и типы, по которым обход идёт дальше.
struct Member {
    name: String,
    attrs: Vec<Attribute>,
    types: Vec<Type>,
}

fn members(item: &Item) -> Option<(String, Vec<Member>)> {
    fn of_fields(fields: &Fields) -> Vec<Type> {
        fields.iter().map(|field| field.ty.clone()).collect()
    }
    match item {
        Item::Struct(item) => Some((
            item.ident.to_string(),
            item.fields
                .iter()
                .map(|field| Member {
                    name: field
                        .ident
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_default(),
                    attrs: field.attrs.clone(),
                    types: vec![field.ty.clone()],
                })
                .collect(),
        )),
        Item::Enum(item) => Some((
            item.ident.to_string(),
            item.variants
                .iter()
                .map(|variant| Member {
                    name: variant.ident.to_string(),
                    attrs: variant.attrs.clone(),
                    types: of_fields(&variant.fields),
                })
                .collect(),
        )),
        _ => None,
    }
}

/// Значения `rename` и `alias` из `#[serde(...)]`.
fn serde_names(attrs: &[Attribute]) -> (Option<String>, Vec<String>) {
    let mut rename = None;
    let mut aliases = Vec::new();
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("serde")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") {
                rename = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.path.is_ident("alias") {
                aliases.push(meta.value()?.parse::<LitStr>()?.value());
            } else if meta.input.peek(syn::Token![=]) {
                meta.value()?.parse::<syn::Expr>()?;
            } else if meta.input.peek(syn::token::Paren) {
                meta.parse_nested_meta(|inner| {
                    if inner.input.peek(syn::Token![=]) {
                        inner.value()?.parse::<syn::Expr>()?;
                    }
                    Ok(())
                })?;
            }
            Ok(())
        })
        .expect("serde attribute");
    }
    (rename, aliases)
}

/// Имена типов, которые встречаются в записи типа, с обобщёнными аргументами.
fn named_types(ty: &Type, names: &mut Vec<String>) {
    struct Collect<'a>(&'a mut Vec<String>);
    impl<'ast> syn::visit::Visit<'ast> for Collect<'_> {
        fn visit_path_segment(&mut self, segment: &'ast syn::PathSegment) {
            self.0.push(segment.ident.to_string());
            syn::visit::visit_path_segment(self, segment);
        }
    }
    syn::visit::Visit::visit_type(&mut Collect(names), ty);
}

fn model_synonyms(root: &Path) -> Vec<ModelSynonym> {
    let mut types = BTreeMap::new();
    for layer in MODEL_LAYERS {
        for file in collect_rust_files(&root.join(layer)) {
            for item in production_items(&file) {
                if let Some((name, members)) = members(&item) {
                    types.insert(name, members);
                }
            }
        }
    }
    assert!(types.contains_key(MODEL_ROOT), "{MODEL_ROOT} is not found");

    let mut synonyms = BTreeSet::new();
    let mut seen = BTreeSet::from([MODEL_ROOT.to_owned()]);
    let mut queue = VecDeque::from([MODEL_ROOT.to_owned()]);
    while let Some(owner) = queue.pop_front() {
        for member in &types[&owner] {
            let (rename, aliases) = serde_names(&member.attrs);
            for alias in aliases {
                let canonical = rename.clone().unwrap_or_else(|| {
                    panic!(
                        "{owner}::{} accepts `{alias}` without an explicit `rename`: name the key the synonym stands for",
                        member.name
                    )
                });
                synonyms.insert(ModelSynonym {
                    owner: owner.clone(),
                    canonical,
                    alias,
                });
            }
            let mut names = Vec::new();
            for ty in &member.types {
                named_types(ty, &mut names);
            }
            for name in names {
                if types.contains_key(&name) && seen.insert(name.clone()) {
                    queue.push_back(name);
                }
            }
        }
    }
    synonyms.into_iter().collect()
}

/// Все объекты `properties` схемы.
fn property_maps<'a>(value: &'a Value, maps: &mut Vec<&'a serde_json::Map<String, Value>>) {
    match value {
        Value::Object(object) => {
            if let Some(Value::Object(properties)) = object.get("properties") {
                maps.push(properties);
            }
            for child in object.values() {
                property_maps(child, maps);
            }
        }
        Value::Array(items) => items.iter().for_each(|item| property_maps(item, maps)),
        _ => {}
    }
}

#[test]
fn every_key_synonym_of_the_model_is_deprecated_in_the_schema() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let synonyms = model_synonyms(root);
    // Модель сегодня принимает прежние имена секции `push:` и ключей `providers.*`; пустой
    // список значил бы, что обход потерял модель, а не что синонимов не стало.
    assert!(
        synonyms
            .iter()
            .any(|synonym| synonym.owner == MODEL_ROOT && synonym.alias == "build"),
        "{synonyms:?}"
    );

    let schemas = SCHEMAS
        .iter()
        .map(|path| {
            let text = std::fs::read_to_string(root.join(path)).expect("schema");
            (*path, serde_json::from_str::<Value>(&text).expect("json"))
        })
        .collect::<Vec<_>>();

    for synonym in &synonyms {
        let mut checked = 0;
        for (path, schema) in &schemas {
            let mut maps = Vec::new();
            property_maps(schema, &mut maps);
            for properties in maps
                .into_iter()
                .filter(|properties| properties.contains_key(&synonym.canonical))
            {
                checked += 1;
                let entry = properties.get(&synonym.alias).unwrap_or_else(|| {
                    panic!(
                        "{path}: `{}` is accepted for `{}` ({}) but is absent next to it",
                        synonym.alias, synonym.canonical, synonym.owner
                    )
                });
                assert_eq!(
                    entry.get("deprecated"),
                    Some(&Value::Bool(true)),
                    "{path}: `{}` ({}) is not deprecated: {entry}",
                    synonym.alias,
                    synonym.owner
                );
            }
        }
        assert!(
            checked > 0,
            "no published schema declares `{}`, so its synonym `{}` ({}) is unchecked",
            synonym.canonical,
            synonym.alias,
            synonym.owner
        );
    }
}

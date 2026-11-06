mod guardrail_support;

use std::path::Path;

use guardrail_support::{collect_rust_files, normalize_tokens, production_tokens};
use syn::visit::Visit;

// Конверт ответа — тоже представление, но живёт отдельно от `output`: без своей строки он
// прошёл бы мимо стража. Образцы ловят путь с `::` — импорт модуля целиком тем же способом
// не виден, как и у остальных строк списка.
const FORBIDDEN_PATTERNS: &[&str] = &[
    "clap::",
    "crate::cli::",
    "crate::output::",
    "crate::command_envelope::",
    "crate::mcp::",
];

fn assert_missing(path: &Path, forbidden: &str) {
    let production = production_tokens(path);
    assert!(
        !production.contains(forbidden),
        "{} must not import {}",
        path.display(),
        forbidden
    );
}

#[test]
fn use_cases_do_not_depend_on_transport_or_presentation_types() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("use_cases");
    let files = collect_rust_files(&root);
    for expected in ["build_project.rs", "result.rs", "workspace_lock.rs"] {
        assert!(
            files
                .iter()
                .any(|path| path.file_name().is_some_and(|name| name == expected)),
            "expected recursive scan to include src/use_cases/{expected}"
        );
    }

    for file in &files {
        for forbidden in FORBIDDEN_PATTERNS {
            assert_missing(file, forbidden);
        }
    }
}

/// Блоки обмениваются только явным контекстом: в слое сценариев нет скрытого общего
/// состояния, через которое один блок мог бы передать другому путь, артефакт или
/// разобранный вывод в обход сигнатур.
#[test]
fn use_cases_keep_no_hidden_shared_state() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("use_cases");
    let context = root.join("context.rs");
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("use_cases dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|value| value.to_str()) != Some("rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("source");
            for forbidden in [
                "static mut",
                "thread_local!",
                "OnceCell",
                "lazy_static!",
                "env::set_var",
            ] {
                assert!(
                    !source.contains(forbidden),
                    "{} passes state through hidden `{forbidden}` instead of a typed context",
                    path.display()
                );
            }
            assert!(
                !once_lock_outside_context(&source, path == context),
                "{} passes state through hidden OnceLock storage instead of a typed context",
                path.display()
            );
        }
    }
}

/// OnceLock в явном контексте — состояние команды, а не скрытый глобальный слот.
/// Из общего числа упоминаний вычитаем только сегменты путей типов полей. Выражения
/// в длинах массивов и константных аргументах остаются под проверкой.
fn once_lock_outside_context(source: &str, is_context_file: bool) -> bool {
    #[derive(Default)]
    struct ContextOnceLocks(usize);

    impl<'ast> Visit<'ast> for ContextOnceLocks {
        fn visit_type_path(&mut self, node: &'ast syn::TypePath) {
            self.0 += node
                .path
                .segments
                .iter()
                .filter(|segment| segment.ident == "OnceLock")
                .count();
            syn::visit::visit_type_path(self, node);
        }

        fn visit_expr(&mut self, _: &'ast syn::Expr) {}
    }

    let file = syn::parse_file(source).expect("parse source");
    let mentions = normalize_tokens(&file).match_indices("OnceLock").count();
    let mut allowed = ContextOnceLocks::default();
    if is_context_file {
        for item in &file.items {
            if let syn::Item::Struct(item) = item {
                if item.ident == "ExecutionContext" {
                    for field in &item.fields {
                        allowed.visit_type(&field.ty);
                    }
                }
            }
        }
    }
    mentions > allowed.0
}

#[test]
fn the_shared_state_guard_allows_only_declared_context_storage() {
    let context = "struct ExecutionContext { session: Arc<OnceLock<Endpoint>> }";
    assert!(!once_lock_outside_context(context, true));
    assert!(once_lock_outside_context(context, false));
    let hidden_in_length = "struct ExecutionContext {
        bytes: [u8; { static SLOT: OnceLock<u8> = OnceLock::new(); 0 }],
    }";
    assert!(
        once_lock_outside_context(hidden_in_length, true),
        "missed hidden storage in an array length"
    );
    for hidden in [
        "static SLOT: OnceLock<Endpoint> = OnceLock::new();",
        "struct Other { slot: OnceLock<Endpoint> }",
        "use std::sync::OnceLock as Slot;",
        "type Slot = std::sync::OnceLock<Endpoint>;",
        "hidden!(static SLOT: OnceLock<Endpoint> = OnceLock::new());",
        "#[hidden(OnceLock)] struct ExecutionContext { session: Endpoint }",
        "struct ExecutionContext { #[hidden(OnceLock)] session: Endpoint }",
    ] {
        assert!(
            once_lock_outside_context(&format!("{context} {hidden}"), true),
            "missed hidden storage: {hidden}"
        );
    }
}

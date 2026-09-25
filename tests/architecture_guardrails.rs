mod guardrail_support;

use regex::Regex;
use std::ffi::OsString;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

use guardrail_support::{
    collect_rust_files, free_function_tokens, has_cfg_test, item_has_cfg_test, normalize_tokens,
    parse_rust_file, production_source, production_tokens,
};

const EXPECTED_MCP_TOOLS: &[&str] = &[
    "run_all_tests",
    "run_module_tests",
    "build_project",
    "dump_config",
    "launch_app",
    "check_syntax_edt",
    "check_syntax_designer_config",
    "check_syntax_designer_modules",
];

const FORBIDDEN_PROCESS_PATTERNS: &[&str] = &[
    "std::process::Command",
    "tokio::process::Command",
    "usestd::process::Command",
    "usestd::process::{Command",
    "usestd::process::Stdio",
    "usestd::process::{Stdio",
    "usestd::process::Child",
    "usestd::process::{Child",
    "usestd::process::ExitStatus",
    "usestd::process::{ExitStatus",
    "Command::new(",
    "Stdio::",
];

fn repo_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn read(relative: &str) -> String {
    fs::read_to_string(repo_path(relative)).expect("read repository file")
}

fn extract_between<'a>(contents: &'a str, start_marker: &str, end_marker: &str) -> &'a str {
    let start = contents
        .find(start_marker)
        .unwrap_or_else(|| panic!("missing marker: {start_marker}"));
    let tail = &contents[start..];
    let end = tail
        .find(end_marker)
        .unwrap_or_else(|| panic!("missing marker: {end_marker}"));
    &tail[..end]
}

fn extract_backticked_items(section: &str) -> Vec<String> {
    let regex = Regex::new(r"`([^`]+)`").expect("regex");
    regex
        .captures_iter(section)
        .map(|capture| capture[1].to_owned())
        .collect()
}

#[test]
fn raw_process_spawn_apis_stay_inside_platform_layer() {
    let root = repo_path("src");
    let files = collect_rust_files(&root);
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src_main = Path::new("src").join("main.rs");
    let src_platform = Path::new("src").join("platform");

    for file in files {
        let relative = file.strip_prefix(repo_root).expect("relative path");
        if relative == src_main || relative.starts_with(&src_platform) {
            continue;
        }

        let production = production_tokens(&file);
        for forbidden in FORBIDDEN_PROCESS_PATTERNS {
            assert!(
                !production.contains(forbidden),
                "{} must keep raw process API '{}' inside src/platform",
                relative.display(),
                forbidden
            );
        }
    }
}

#[test]
fn mcp_surface_snapshot_stays_explicit_and_documented() {
    let source = read("src/mcp/server.rs");
    let source_section = extract_between(
        &source,
        "const fn as_str(self) -> &'static str {",
        "fn admission_timeout",
    );
    let source_tools = Regex::new(r#""([a-z_]+)""#)
        .expect("regex")
        .captures_iter(source_section)
        .map(|capture| capture[1].to_owned())
        .collect::<Vec<_>>();

    let contract = read("spec/rules/mcp/published-tool-surface.md");
    // Числительное прозы сверяется со счётом, а не служит якорем: прежде девятый
    // инструмент с обновлённым перечнем и прежней прозой проходил молча.
    let published = contract
        .lines()
        .find(|line| line.starts_with("Опубликованы ") && line.ends_with(" инструментов:"))
        .expect("contract announces the published tools");
    let counted = published
        .trim_start_matches("Опубликованы ")
        .trim_end_matches(" инструментов:");
    let contract_section = extract_between(
        &contract,
        published,
        "Состав меняется только вместе с версией этой формы.",
    );
    let contract_tools = extract_backticked_items(contract_section);
    assert_eq!(
        counted,
        russian_numeral(contract_tools.len()),
        "проза называет другое число инструментов, чем перечисляет"
    );

    let expected = EXPECTED_MCP_TOOLS
        .iter()
        .map(|tool| (*tool).to_owned())
        .collect::<Vec<_>>();

    assert_eq!(source_tools, expected);
    assert_eq!(contract_tools, expected);
}

/// Сценарий уходит в работу только под замком `workPath`, и держит это сам код, а не
/// перечень адаптеров, записанный руками.
///
/// Как читается код:
/// - сценарий — свободная функция модуля `use_cases` или `mcp::edt_syntax` (единственного
///   сценария, который живёт в адаптере, — arc42 §5.1, §6.7); ссылка на него — любой путь
///   в выражении: вызов или указатель на функцию, после разрешения через `use` модуля и
///   функции, `crate`, `self` и `super`. Типы и их методы (`ExecutionContext::cli`) —
///   не сценарии;
/// - помощник замка узнаётся по устройству, а не по имени: он берёт замок
///   (`acquire_workspace_lock` или функцию, которая зовёт его сама) и зовёт переданное ему
///   замыкание либо отдаёт его другому помощнику; область под замком — замыкания в
///   аргументах вызова помощника;
/// - ссылка вне слоя сценариев лежит в области под замком либо внутри свободной функции,
///   на которую ссылаются только из областей под замком (одна ступень);
/// - сценарии, которые по правилу идут без замка, названы ниже поимённо — с причиной.
///
/// Не видит: пути внутри макросов, реэкспорт модуля (реэкспорт функции прослежен),
/// сценарий, вызванный через трейт или метод, и помощника, который отпускает замок до
/// вызова замыкания. Замыкание, вызванное раньше захвата, помощника не делает. Точный счёт ссылок ловит ссылку, ушедшую в макрос;
/// как проверка читает код, держит `the_lock_guard_tells_locked_dispatches_from_unlocked_ones`.
#[test]
fn every_scenario_is_dispatched_under_the_workspace_lock() {
    let report = LockBoundaryReport::of(&SourceIndex::of_src(), UNLOCKED_SCENARIOS);

    assert!(
        report.violations.is_empty(),
        "a scenario is dispatched outside the workspace lock:\n{}",
        report.violations.join("\n")
    );
    let unused = UNLOCKED_SCENARIOS
        .iter()
        .map(|(path, _)| *path)
        .filter(|path| !report.exempted.contains(*path))
        .collect::<Vec<_>>();
    assert!(
        unused.is_empty(),
        "an exemption names a scenario no adapter reaches any more: {unused:?}"
    );
    assert_eq!(
        report.accepted, 24,
        "the number of locked dispatches changed: update it when a command is added or removed, \
         or find the dispatch that moved out of the guard's sight"
    );
    for module in [
        "crate::app",
        "crate::cli::execute",
        "crate::mcp::port",
        "crate::mcp::server",
    ] {
        assert!(
            report.accepted_in.contains(module),
            "no locked dispatch seen in {module}: {:?}",
            report.accepted_in
        );
    }
}

/// Как проверка замка читает код — на исходниках, где ответ известен заранее: вызов в
/// замыкании помощника принят, прямой вызов, вызов через `use … as` и указатель на функцию
/// — нет; функция, которую зовут только под замком, принята, а зовут ещё и без него — нет;
/// «помощник», который замка не берёт, никого не прикрывает.
#[test]
fn the_lock_guard_tells_locked_dispatches_from_unlocked_ones() {
    let index = SourceIndex::from_sources(&[
        (
            "crate::use_cases::workspace_lock",
            "pub(crate) fn acquire_workspace_lock() -> Guard { Guard }",
        ),
        (
            "crate::use_cases::transport",
            "use crate::use_cases::workspace_lock::acquire_workspace_lock;\n\
             fn acquire() -> Guard { acquire_workspace_lock() }\n\
             pub fn with_lock<T>(run: impl FnOnce() -> T) -> T { let _guard = acquire(); run() }\n\
             pub fn with_lock_too<T>(run: impl FnOnce() -> T) -> T { with_lock(run) }\n\
             pub fn pretends<T>(run: impl FnOnce() -> T) -> T { run() }\n\
             pub fn runs_first<T>(run: impl FnOnce() -> T) -> T { let value = run(); let _guard = acquire(); value }",
        ),
        ("crate::use_cases::dump", "pub fn execute() {}"),
        (
            "crate::cli::adapter",
            "use crate::use_cases::dump;\n\
             use crate::use_cases::transport::{pretends, runs_first, with_lock, with_lock_too};\n\
             fn locked() { with_lock(|| dump::execute()); }\n\
             fn locked_too() { with_lock_too(|| crate::use_cases::dump::execute()); }\n\
             fn only_under_the_lock() { dump::execute(); }\n\
             fn caller() { with_lock(|| only_under_the_lock()); }\n\
             fn also_unlocked() { dump::execute(); }\n\
             fn first() { with_lock(|| also_unlocked()); }\n\
             fn second() { also_unlocked(); }\n\
             fn direct() { dump::execute(); }\n\
             fn renamed() { use crate::use_cases::dump::execute as run_now; run_now(); }\n\
             fn pointer() { let run: fn() = dump::execute; with_lock(run); }\n\
             fn fake() { pretends(|| dump::execute()); }\n\
             fn early() { runs_first(|| dump::execute()); }\n\
             fn globbed() { use crate::use_cases::dump::*; execute(); }",
        ),
    ]);

    let report = LockBoundaryReport::of(&index, &[]);

    let mut unlocked = report
        .violations
        .iter()
        .map(|violation| {
            let context = violation.split(['(', ')']).nth(1).unwrap_or_default();
            let target = violation.split('`').nth(1).unwrap_or_default();
            format!("{context} {target}")
        })
        .collect::<Vec<_>>();
    unlocked.sort();
    assert_eq!(
        unlocked,
        [
            "also_unlocked crate::use_cases::dump::execute",
            "direct crate::use_cases::dump::execute",
            "early crate::use_cases::dump::execute",
            "early crate::use_cases::transport::runs_first",
            "fake crate::use_cases::dump::execute",
            "fake crate::use_cases::transport::pretends",
            "globbed crate::use_cases::dump",
            "pointer crate::use_cases::dump::execute",
            "renamed crate::use_cases::dump::execute",
        ]
    );
    assert_eq!(report.accepted, 3, "{:?}", report.violations);
}

/// Сценарии, которые по правилу идут без замка `workPath`, с причиной. Запись, которой
/// больше никто не пользуется, валит проверку.
const UNLOCKED_SCENARIOS: &[(&str, &str)] = &[
    (
        "crate::use_cases::config_init::execute",
        "`init` пишет описание проекта, в `workPath` ничего",
    ),
    (
        "crate::use_cases::bootstrap_project::plan",
        "план клона ничего не пишет; замок берётся по нему",
    ),
    (
        "crate::use_cases::configure_extensions::resolve_targets",
        "проверка запроса до замка",
    ),
    (
        "crate::use_cases::convert_sources::preflight_validate",
        "проверка запроса до замка",
    ),
    (
        "crate::use_cases::infobase_export::validate_configuration_request",
        "проверка запроса до замка",
    ),
    (
        "crate::use_cases::infobase_export::validate_snapshot_output",
        "проверка запроса до замка",
    ),
    (
        "crate::use_cases::infobase_export::validate_restore_request",
        "проверка запроса до замка",
    ),
    (
        "crate::use_cases::infobase_export::prepare_configuration_export",
        "выбор исполнителя до замка: превью возвращается раньше него",
    ),
    (
        "crate::use_cases::infobase_export::prepare_infobase_snapshot",
        "выбор исполнителя до замка: превью возвращается раньше него",
    ),
    (
        "crate::use_cases::infobase_export::prepare_infobase_restore",
        "выбор исполнителя до замка: превью возвращается раньше него",
    ),
    (
        "crate::use_cases::infobase_export::preview_configuration_export",
        "превью замка не берёт",
    ),
    (
        "crate::use_cases::infobase_export::preview_infobase_snapshot",
        "превью замка не берёт",
    ),
    (
        "crate::use_cases::infobase_export::preview_infobase_restore",
        "превью замка не берёт",
    ),
    (
        "crate::use_cases::request::effective_test_timeouts",
        "чистая функция над запросом",
    ),
];

/// Подключение `ibcmd` — а с ним требование секции `infobase.dbms` — строится только
/// там, где сценарий зовёт `ibcmd`: исполнителем после выбора или для пробы. Перечень
/// мест закрыт: новое место валит проверку, пока ревью не решит, что оно после выбора
/// исполнителя, а не до него.
#[test]
fn an_ibcmd_connection_is_built_only_where_ibcmd_runs() {
    const BUILT_FOR_IBCMD: &[(&str, &str)] = &[
        (
            "crate::use_cases::extension_inventory::Executor::of",
            "исполнитель `extensions`: ветка `ibcmd` после выбора",
        ),
        (
            "crate::use_cases::load_artifact::installed_extension_state",
            "проба расширения перед `upload`",
        ),
        (
            "crate::use_cases::infobase_export::readiness",
            "готовность кандидата `ibcmd` в его же переборе",
        ),
        (
            "crate::use_cases::infobase_export::run_configuration_provider",
            "ветка `ibcmd` после выбора",
        ),
        (
            "crate::use_cases::init_project::create_infobase_via_ibcmd",
            "создание базы, когда выбран `ibcmd`",
        ),
        (
            "crate::use_cases::build_project::helpers::build_ibcmd_dsl",
            "шаг `ibcmd` сборки",
        ),
        (
            "crate::use_cases::dump_config::helpers::build_ibcmd_dsl",
            "шаг `ibcmd` выгрузки",
        ),
        (
            "crate::use_cases::tool_extension::build_ibcmd_dsl",
            "расширение-инструмент, когда сборку ведёт `ibcmd`",
        ),
    ];
    let expected = BUILT_FOR_IBCMD
        .iter()
        .map(|(site, _)| (*site).to_owned())
        .collect::<std::collections::BTreeSet<_>>();

    assert_eq!(
        ibcmd_connection_sites(&SourceIndex::of_src()),
        expected,
        "the places that build an ibcmd connection changed; each must run after ibcmd was selected or be its probe"
    );
}

/// Место постройки называет модуль, тип и метод: одноимённые методы разных типов не
/// сливаются в одно место, модуль внутри файла прочитан вместе с ним, а обёртка через
/// `Self::` в самом `ibcmd.rs` — тоже место.
#[test]
fn the_ibcmd_site_finder_names_the_type_and_reads_nested_modules() {
    let index = SourceIndex::from_sources(&[
        (
            "crate::platform::ibcmd",
            "pub struct IbcmdConnection;\n\
             impl IbcmdConnection {\n\
                 pub fn from_infobase() -> Self { Self }\n\
                 pub fn wrapped() -> Self { Self::from_infobase() }\n\
             }",
        ),
        (
            "crate::use_cases::family",
            "use crate::platform::ibcmd::IbcmdConnection;\n\
             struct Executor;\n\
             impl Executor { fn of() { IbcmdConnection::from_infobase(); } }\n\
             struct Probe;\n\
             impl Probe { fn of() {} }\n\
             mod inner { fn eager() { crate::platform::ibcmd::IbcmdConnection::from_infobase(); } }",
        ),
    ]);

    assert_eq!(
        ibcmd_connection_sites(&index),
        [
            "crate::platform::ibcmd::IbcmdConnection::wrapped",
            "crate::use_cases::family::Executor::of",
            "crate::use_cases::family::inner::eager",
        ]
        .map(str::to_owned)
        .into_iter()
        .collect()
    );
}

fn ibcmd_connection_sites(index: &SourceIndex) -> std::collections::BTreeSet<String> {
    let constructor = path_of("crate::platform::ibcmd::IbcmdConnection::from_infobase");
    production_bodies(index)
        .into_iter()
        .filter(|body| {
            let mut finder = PathFinder {
                index,
                module: &body.module,
                local_uses: body.local_uses(index),
                target: &constructor,
                found: false,
            };
            syn::visit::visit_block(&mut finder, body.block);
            finder.found
        })
        .map(|body| format!("{}::{}", body.module.join("::"), body.context))
        .collect()
}

fn path_of(path: &str) -> Vec<String> {
    path.split("::").map(str::to_owned).collect()
}

/// Итог проверки замка: что нарушено, что принято и где, какие исключения пригодились.
#[derive(Default)]
struct LockBoundaryReport {
    violations: Vec<String>,
    accepted: usize,
    accepted_in: std::collections::BTreeSet<String>,
    exempted: std::collections::BTreeSet<String>,
}

impl LockBoundaryReport {
    fn of(index: &SourceIndex, exemptions: &[(&str, &str)]) -> Self {
        let lock = index.lock_api();
        let mut scan = DispatchScan::default();
        for body in production_bodies(index) {
            if body.unit.in_scenario_layer() {
                continue;
            }
            for glob in body
                .globs
                .iter()
                .chain(&block_uses(index, &body.module, body.block).globs)
            {
                if is_scenario_module(glob) {
                    scan.violations.push(format!(
                        "{} ({}): a glob import from `{}` hides what it dispatches",
                        body.unit.file.display(),
                        body.context,
                        glob.join("::")
                    ));
                }
            }
            let mut walker = DispatchWalker {
                index,
                helpers: &lock.helpers,
                body: &body,
                local_uses: body.local_uses(index),
                locked: 0,
                scan: &mut scan,
            };
            syn::visit::visit_block(&mut walker, body.block);
        }

        // Ссылки на каждую функцию: все ли под замком.
        let mut callers: std::collections::HashMap<&[String], bool> =
            std::collections::HashMap::new();
        for (target, locked) in &scan.function_references {
            *callers.entry(target.as_slice()).or_insert(true) &= *locked;
        }

        let mut report = Self {
            violations: std::mem::take(&mut scan.violations),
            ..Self::default()
        };
        for reference in &scan.scenario_references {
            let target = reference.target.join("::");
            if exemptions.iter().any(|(path, _)| *path == target) {
                report.exempted.insert(target);
                continue;
            }
            let covered = reference.locked
                || reference
                    .enclosing
                    .as_deref()
                    .and_then(|enclosing| callers.get(enclosing))
                    .copied()
                    .unwrap_or(false);
            if covered {
                report.accepted += 1;
                report.accepted_in.insert(reference.module.join("::"));
            } else {
                report.violations.push(format!(
                    "{} ({}): `{target}` runs without the workspace lock",
                    reference.file.display(),
                    reference.context
                ));
            }
        }
        report
    }
}

/// Один исходный файл программы: его модуль и разобранное дерево.
struct SourceUnit {
    file: PathBuf,
    module: Vec<String>,
    syntax: syn::File,
}

impl SourceUnit {
    fn in_scenario_layer(&self) -> bool {
        is_scenario_module(&self.module)
    }
}

/// Производственный код: модули, их `use` и свободные функции.
struct SourceIndex {
    units: Vec<SourceUnit>,
    /// Импорты модуля: имя → полный путь.
    uses: std::collections::HashMap<Vec<String>, std::collections::HashMap<String, Vec<String>>>,
    /// Имена, объявленные в модуле: функции, модули, типы.
    items: std::collections::HashMap<Vec<String>, std::collections::HashSet<String>>,
    /// Свободные функции по полному пути.
    functions: std::collections::HashMap<Vec<String>, IndexedFunction>,
}

struct IndexedFunction {
    module: Vec<String>,
    item: syn::ItemFn,
}

impl SourceIndex {
    fn of_src() -> Self {
        let root = repo_path("src");
        let units = collect_rust_files(&root)
            .into_iter()
            .map(|file| {
                let relative = file.strip_prefix(&root).expect("under src");
                let mut parts = relative
                    .iter()
                    .map(|part| part.to_string_lossy().into_owned())
                    .collect::<Vec<_>>();
                let last = parts.pop().expect("file name");
                let mut module = vec!["crate".to_owned()];
                module.extend(parts);
                match last.as_str() {
                    "main.rs" | "mod.rs" => {}
                    name => module.push(name.trim_end_matches(".rs").to_owned()),
                }
                let syntax = parse_rust_file(&file);
                SourceUnit {
                    file,
                    module,
                    syntax,
                }
            })
            .collect();
        Self::build(units)
    }

    /// Индекс исходников, заданных строками: модуль и текст.
    fn from_sources(sources: &[(&str, &str)]) -> Self {
        let units = sources
            .iter()
            .map(|(module, source)| SourceUnit {
                file: PathBuf::from(format!("{}.rs", module.replace("::", "/"))),
                module: path_of(module),
                syntax: syn::parse_file(source).expect("parse source"),
            })
            .collect();
        Self::build(units)
    }

    /// Сначала имена всех модулей, затем `use`: относительный `use` разрешается по именам
    /// своего модуля, где бы они ни стояли.
    fn build(units: Vec<SourceUnit>) -> Self {
        let mut index = Self {
            units: Vec::new(),
            uses: Default::default(),
            items: Default::default(),
            functions: Default::default(),
        };
        for unit in &units {
            index.index_names(&unit.module, &unit.syntax.items);
        }
        for unit in &units {
            index.index_uses(&unit.module, &unit.syntax.items);
        }
        index.units = units;
        index
    }

    fn index_names(&mut self, module: &[String], items: &[syn::Item]) {
        for item in items {
            if item_has_cfg_test(item) {
                continue;
            }
            let name = match item {
                syn::Item::Fn(item_fn) => {
                    let name = item_fn.sig.ident.to_string();
                    self.functions.insert(
                        [module, std::slice::from_ref(&name)].concat(),
                        IndexedFunction {
                            module: module.to_vec(),
                            item: item_fn.clone(),
                        },
                    );
                    Some(name)
                }
                syn::Item::Mod(item_mod) => {
                    let name = item_mod.ident.to_string();
                    if let Some((_, nested)) = &item_mod.content {
                        self.index_names(&[module, std::slice::from_ref(&name)].concat(), nested);
                    }
                    Some(name)
                }
                syn::Item::Struct(item) => Some(item.ident.to_string()),
                syn::Item::Enum(item) => Some(item.ident.to_string()),
                syn::Item::Const(item) => Some(item.ident.to_string()),
                syn::Item::Static(item) => Some(item.ident.to_string()),
                syn::Item::Trait(item) => Some(item.ident.to_string()),
                syn::Item::Type(item) => Some(item.ident.to_string()),
                _ => None,
            };
            if let Some(name) = name {
                self.items.entry(module.to_vec()).or_default().insert(name);
            }
        }
    }

    fn index_uses(&mut self, module: &[String], items: &[syn::Item]) {
        for item in items {
            if item_has_cfg_test(item) {
                continue;
            }
            match item {
                syn::Item::Use(item_use) => {
                    for (alias, path) in flatten_use(&item_use.tree).names {
                        let full = self.absolute(module, &path);
                        self.uses
                            .entry(module.to_vec())
                            .or_default()
                            .insert(alias, full);
                    }
                }
                syn::Item::Mod(item_mod) => {
                    if let Some((_, nested)) = &item_mod.content {
                        let inner = [module, &[item_mod.ident.to_string()]].concat();
                        self.index_uses(&inner, nested);
                    }
                }
                _ => {}
            }
        }
    }

    /// Путь из `use` как полный: `crate`, `self`, `super` или имя, объявленное в модуле.
    fn absolute(&self, module: &[String], path: &[String]) -> Vec<String> {
        match path.first().map(String::as_str) {
            Some("crate") => path.to_vec(),
            Some("self") => [module, &path[1..]].concat(),
            Some("super") => {
                let mut base = module.to_vec();
                let mut rest = path;
                while rest.first().map(String::as_str) == Some("super") {
                    base.pop();
                    rest = &rest[1..];
                }
                [base.as_slice(), rest].concat()
            }
            Some(first)
                if self
                    .items
                    .get(module)
                    .is_some_and(|items| items.contains(first)) =>
            {
                [module, path].concat()
            }
            _ => path.to_vec(),
        }
    }

    /// Полный путь выражения: через `use` функции, затем модуля, затем имена модуля. Одно
    /// слово `self`, `super` или `crate` — значение, а не путь к функции.
    fn resolve(
        &self,
        module: &[String],
        local_uses: &std::collections::HashMap<String, Vec<String>>,
        path: &syn::Path,
    ) -> Option<Vec<String>> {
        let segments = path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>();
        let first = segments.first()?.as_str();
        match first {
            "crate" | "self" | "super" if segments.len() == 1 => None,
            "crate" | "self" | "super" => Some(self.absolute(module, &segments)),
            _ => {
                if let Some(prefix) = local_uses
                    .get(first)
                    .or_else(|| self.uses.get(module).and_then(|uses| uses.get(first)))
                {
                    return Some([prefix.as_slice(), &segments[1..]].concat());
                }
                self.items
                    .get(module)
                    .is_some_and(|items| items.contains(first))
                    .then(|| [module, segments.as_slice()].concat())
            }
        }
    }

    /// Путь выражения после реэкспорта функции: помощник или захват под чужим именем —
    /// всё тот же помощник. Путь не к свободной функции — к методу типа — остаётся как есть.
    fn resolve_target(
        &self,
        module: &[String],
        local_uses: &std::collections::HashMap<String, Vec<String>>,
        path: &syn::Path,
    ) -> Option<Vec<String>> {
        self.resolve(module, local_uses, path)
            .map(|path| self.function_at(path.clone()).unwrap_or(path))
    }

    /// Свободная функция, куда ведёт путь, — через `use` модуля-владельца, если имя в нём
    /// лишь реэкспорт. Путь к модулю, типу или переменной функцией не считается.
    fn function_at(&self, path: Vec<String>) -> Option<Vec<String>> {
        let mut path = path;
        for _ in 0..4 {
            if self.functions.contains_key(&path) {
                return Some(path);
            }
            let (name, owner) = path.split_last()?;
            path = self.uses.get(owner)?.get(name)?.clone();
        }
        None
    }

    /// Замок по устройству кода. Берёт его `acquire_workspace_lock` или функция без
    /// замыканий, которая зовёт его сама; помощник с замыканием берёт замок и зовёт своё
    /// замыкание либо отдаёт его другому помощнику. Точка неподвижная: помощники зовут
    /// друг друга.
    fn lock_api(&self) -> LockApi {
        let acquire = path_of("crate::use_cases::workspace_lock::acquire_workspace_lock");
        let mut acquirers = std::collections::HashSet::from([acquire.clone()]);
        for (path, function) in &self.functions {
            if closure_parameters(&function.item.sig).is_empty()
                && self.body_references(function, &acquire)
            {
                acquirers.insert(path.clone());
            }
        }
        let mut helpers = std::collections::HashSet::new();
        loop {
            let before = helpers.len();
            for (path, function) in &self.functions {
                if helpers.contains(path) {
                    continue;
                }
                let closures = closure_parameters(&function.item.sig);
                if closures.is_empty() {
                    continue;
                }
                let mut probe = HelperProbe {
                    index: self,
                    module: &function.module,
                    local_uses: block_local_uses(self, &function.module, &function.item.block),
                    closures: &closures,
                    helpers: &helpers,
                    acquirers: &acquirers,
                    takes_lock: false,
                    calls_closure: false,
                    hands_closure_on: false,
                    inside_helper_call: 0,
                };
                syn::visit::visit_block(&mut probe, &function.item.block);
                if (probe.takes_lock && probe.calls_closure) || probe.hands_closure_on {
                    helpers.insert(path.clone());
                }
            }
            if helpers.len() == before {
                return LockApi { acquirers, helpers };
            }
        }
    }

    fn body_references(&self, function: &IndexedFunction, target: &[String]) -> bool {
        let mut finder = PathFinder {
            index: self,
            module: &function.module,
            local_uses: block_local_uses(self, &function.module, &function.item.block),
            target,
            found: false,
        };
        syn::visit::visit_block(&mut finder, &function.item.block);
        finder.found
    }
}

/// Кто берёт замок: функции захвата и помощники с замыканием.
struct LockApi {
    acquirers: std::collections::HashSet<Vec<String>>,
    helpers: std::collections::HashSet<Vec<String>>,
}

/// Тело производственного кода: где лежит, как называется в отчёте, какая свободная
/// функция его объемлет и какие звёздочки импортирует его модуль.
struct Body<'a> {
    unit: &'a SourceUnit,
    module: Vec<String>,
    /// Имя функции или `Тип::метод`.
    context: String,
    /// Полный путь свободной функции; у метода его нет.
    enclosing: Option<Vec<String>>,
    /// Тип, чей это метод: `Self::` в теле ведёт к нему.
    owner: Option<String>,
    block: &'a syn::Block,
    globs: Vec<Vec<String>>,
}

impl Body<'_> {
    /// `use` тела и `Self` метода.
    fn local_uses(&self, index: &SourceIndex) -> std::collections::HashMap<String, Vec<String>> {
        let mut uses = block_local_uses(index, &self.module, self.block);
        if let Some(owner) = &self.owner {
            uses.insert(
                "Self".to_owned(),
                [self.module.as_slice(), std::slice::from_ref(owner)].concat(),
            );
        }
        uses
    }
}

/// Все тела производственного кода: свободные функции, методы типов и методы трейтов по
/// умолчанию, в том числе в модулях внутри файла. Один обход на обе проверки.
fn production_bodies(index: &SourceIndex) -> Vec<Body<'_>> {
    fn walk<'a>(
        index: &SourceIndex,
        unit: &'a SourceUnit,
        module: &[String],
        items: &'a [syn::Item],
        bodies: &mut Vec<Body<'a>>,
    ) {
        let globs = items
            .iter()
            .filter(|item| !item_has_cfg_test(item))
            .filter_map(|item| match item {
                syn::Item::Use(item_use) => Some(flatten_use(&item_use.tree).globs),
                _ => None,
            })
            .flatten()
            .map(|glob| index.absolute(module, &glob))
            .collect::<Vec<_>>();
        let mut push = |context: String,
                        enclosing: Option<Vec<String>>,
                        owner: Option<String>,
                        block: &'a syn::Block| {
            bodies.push(Body {
                unit,
                module: module.to_vec(),
                context,
                enclosing,
                owner,
                block,
                globs: globs.clone(),
            });
        };
        let mut nested_modules = Vec::new();
        for item in items {
            if item_has_cfg_test(item) {
                continue;
            }
            match item {
                syn::Item::Fn(item_fn) => {
                    let name = item_fn.sig.ident.to_string();
                    push(
                        name.clone(),
                        Some([module, &[name]].concat()),
                        None,
                        &item_fn.block,
                    );
                }
                syn::Item::Impl(item_impl) => {
                    let owner = match item_impl.self_ty.as_ref() {
                        syn::Type::Path(type_path) => type_path
                            .path
                            .segments
                            .last()
                            .map(|segment| segment.ident.to_string()),
                        _ => None,
                    }
                    .unwrap_or_else(|| "_".to_owned());
                    for impl_item in &item_impl.items {
                        if let syn::ImplItem::Fn(method) = impl_item {
                            if !has_cfg_test(&method.attrs) {
                                push(
                                    format!("{owner}::{}", method.sig.ident),
                                    None,
                                    Some(owner.clone()),
                                    &method.block,
                                );
                            }
                        }
                    }
                }
                syn::Item::Trait(item_trait) => {
                    for trait_item in &item_trait.items {
                        if let syn::TraitItem::Fn(method) = trait_item {
                            if let Some(block) = &method.default {
                                if !has_cfg_test(&method.attrs) {
                                    push(
                                        format!("{}::{}", item_trait.ident, method.sig.ident),
                                        None,
                                        None,
                                        block,
                                    );
                                }
                            }
                        }
                    }
                }
                syn::Item::Mod(item_mod) => {
                    if let Some((_, nested)) = &item_mod.content {
                        nested_modules
                            .push(([module, &[item_mod.ident.to_string()]].concat(), nested));
                    }
                }
                _ => {}
            }
        }
        for (inner, nested) in nested_modules {
            walk(index, unit, &inner, nested, bodies);
        }
    }

    let mut bodies = Vec::new();
    for unit in &index.units {
        walk(index, unit, &unit.module, &unit.syntax.items, &mut bodies);
    }
    bodies
}

/// Имена и звёздочки одного `use`: `self` в группе — сам префикс, переименование — своим
/// именем.
#[derive(Default)]
struct FlatUse {
    names: Vec<(String, Vec<String>)>,
    globs: Vec<Vec<String>>,
}

fn flatten_use(tree: &syn::UseTree) -> FlatUse {
    fn walk(tree: &syn::UseTree, prefix: &mut Vec<String>, out: &mut FlatUse) {
        match tree {
            syn::UseTree::Path(path) => {
                prefix.push(path.ident.to_string());
                walk(&path.tree, prefix, out);
                prefix.pop();
            }
            syn::UseTree::Name(name) if name.ident == "self" => {
                if let Some(last) = prefix.last() {
                    out.names.push((last.clone(), prefix.clone()));
                }
            }
            syn::UseTree::Name(name) => {
                let mut path = prefix.clone();
                path.push(name.ident.to_string());
                out.names.push((name.ident.to_string(), path));
            }
            syn::UseTree::Rename(rename) => {
                let mut path = prefix.clone();
                if rename.ident != "self" {
                    path.push(rename.ident.to_string());
                }
                out.names.push((rename.rename.to_string(), path));
            }
            syn::UseTree::Glob(_) => out.globs.push(prefix.clone()),
            syn::UseTree::Group(group) => {
                for item in &group.items {
                    walk(item, prefix, out);
                }
            }
        }
    }
    let mut out = FlatUse::default();
    walk(tree, &mut Vec::new(), &mut out);
    out
}

/// `use` внутри тела — на всё тело сразу: область видимости блока проверке не нужна.
fn block_local_uses(
    index: &SourceIndex,
    module: &[String],
    block: &syn::Block,
) -> std::collections::HashMap<String, Vec<String>> {
    block_uses(index, module, block).names
}

/// Разрешённые `use` тела: имена и звёздочки.
struct BlockUses {
    names: std::collections::HashMap<String, Vec<String>>,
    globs: Vec<Vec<String>>,
}

fn block_uses(index: &SourceIndex, module: &[String], block: &syn::Block) -> BlockUses {
    struct Uses<'a> {
        index: &'a SourceIndex,
        module: &'a [String],
        found: BlockUses,
    }
    impl<'ast> syn::visit::Visit<'ast> for Uses<'_> {
        fn visit_item_use(&mut self, node: &'ast syn::ItemUse) {
            let flat = flatten_use(&node.tree);
            for (alias, path) in flat.names {
                let full = self.index.absolute(self.module, &path);
                self.found.names.insert(alias, full);
            }
            for glob in flat.globs {
                let full = self.index.absolute(self.module, &glob);
                self.found.globs.push(full);
            }
        }
    }
    let mut uses = Uses {
        index,
        module,
        found: BlockUses {
            names: Default::default(),
            globs: Vec::new(),
        },
    };
    syn::visit::visit_block(&mut uses, block);
    uses.found
}

/// Имена параметров-замыканий: `impl Fn*` или параметр типа, ограниченный `Fn*`.
fn closure_parameters(sig: &syn::Signature) -> Vec<String> {
    fn is_fn_bound(bound: &syn::TypeParamBound) -> bool {
        matches!(bound, syn::TypeParamBound::Trait(trait_bound)
        if trait_bound.path.segments.last().is_some_and(|segment| {
            matches!(segment.ident.to_string().as_str(), "Fn" | "FnMut" | "FnOnce")
        }))
    }
    let mut closure_types = std::collections::HashSet::new();
    for param in &sig.generics.params {
        if let syn::GenericParam::Type(type_param) = param {
            if type_param.bounds.iter().any(is_fn_bound) {
                closure_types.insert(type_param.ident.to_string());
            }
        }
    }
    if let Some(where_clause) = &sig.generics.where_clause {
        for predicate in &where_clause.predicates {
            if let syn::WherePredicate::Type(predicate) = predicate {
                if predicate.bounds.iter().any(is_fn_bound) {
                    if let syn::Type::Path(bounded) = &predicate.bounded_ty {
                        if let Some(ident) = bounded.path.get_ident() {
                            closure_types.insert(ident.to_string());
                        }
                    }
                }
            }
        }
    }
    sig.inputs
        .iter()
        .filter_map(|input| match input {
            syn::FnArg::Typed(typed) => Some(typed),
            syn::FnArg::Receiver(_) => None,
        })
        .filter_map(|typed| {
            let syn::Pat::Ident(name) = typed.pat.as_ref() else {
                return None;
            };
            let is_closure = match typed.ty.as_ref() {
                syn::Type::ImplTrait(impl_trait) => impl_trait.bounds.iter().any(is_fn_bound),
                syn::Type::Path(type_path) => type_path
                    .path
                    .get_ident()
                    .is_some_and(|ident| closure_types.contains(&ident.to_string())),
                _ => false,
            };
            is_closure.then(|| name.ident.to_string())
        })
        .collect()
}

/// Что делает функция с замком и своими замыканиями.
struct HelperProbe<'a> {
    index: &'a SourceIndex,
    module: &'a [String],
    local_uses: std::collections::HashMap<String, Vec<String>>,
    closures: &'a [String],
    helpers: &'a std::collections::HashSet<Vec<String>>,
    acquirers: &'a std::collections::HashSet<Vec<String>>,
    takes_lock: bool,
    calls_closure: bool,
    hands_closure_on: bool,
    inside_helper_call: usize,
}

impl HelperProbe<'_> {
    fn is_closure_parameter(&self, expr: &syn::Expr) -> bool {
        matches!(expr, syn::Expr::Path(path)
            if path.path.get_ident().is_some_and(|ident| self.closures.contains(&ident.to_string())))
    }
}

impl<'ast> syn::visit::Visit<'ast> for HelperProbe<'_> {
    fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
        if self
            .index
            .resolve_target(self.module, &self.local_uses, &node.path)
            .is_some_and(|target| self.acquirers.contains(&target))
        {
            self.takes_lock = true;
        }
        syn::visit::visit_expr_path(self, node);
    }

    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        // Замыкание засчитано, только если зовётся после захвата: код читается в порядке
        // текста, и вызов до захвата замка ничем не прикрыт.
        if self.is_closure_parameter(&node.func) {
            self.calls_closure |= self.takes_lock;
            if self.inside_helper_call > 0 {
                self.hands_closure_on = true;
            }
        }
        let calls_helper = matches!(node.func.as_ref(), syn::Expr::Path(path)
            if self.index.resolve_target(self.module, &self.local_uses, &path.path)
                .is_some_and(|target| self.helpers.contains(&target)));
        if calls_helper {
            if node.args.iter().any(|arg| self.is_closure_parameter(arg)) {
                self.hands_closure_on = true;
            }
            self.inside_helper_call += 1;
            syn::visit::visit_expr_call(self, node);
            self.inside_helper_call -= 1;
        } else {
            syn::visit::visit_expr_call(self, node);
        }
    }
}

/// Есть ли в теле путь, ведущий к `target`.
struct PathFinder<'a> {
    index: &'a SourceIndex,
    module: &'a [String],
    local_uses: std::collections::HashMap<String, Vec<String>>,
    target: &'a [String],
    found: bool,
}

impl<'ast> syn::visit::Visit<'ast> for PathFinder<'_> {
    fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
        if self
            .index
            .resolve_target(self.module, &self.local_uses, &node.path)
            .is_some_and(|path| path == self.target)
        {
            self.found = true;
        }
        syn::visit::visit_expr_path(self, node);
    }
}

/// Ссылка на сценарий: куда ведёт, откуда и под замком ли.
struct ScenarioReference {
    target: Vec<String>,
    file: PathBuf,
    module: Vec<String>,
    context: String,
    enclosing: Option<Vec<String>>,
    locked: bool,
}

#[derive(Default)]
struct DispatchScan {
    scenario_references: Vec<ScenarioReference>,
    /// Ссылки на свободные функции программы: куда и под замком ли.
    function_references: Vec<(Vec<String>, bool)>,
    violations: Vec<String>,
}

fn is_scenario_module(path: &[String]) -> bool {
    path.starts_with(&path_of("crate::use_cases"))
        || path.starts_with(&path_of("crate::mcp::edt_syntax"))
}

/// Свободная функция сценария: модульный путь и имя в `snake_case`, без типов.
fn is_scenario_function(path: &[String]) -> bool {
    is_scenario_module(path)
        && path.len() > 2
        && path[1..].iter().all(|segment| {
            segment
                .chars()
                .next()
                .is_some_and(|first| first.is_ascii_lowercase())
                && segment
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
}

struct DispatchWalker<'a, 'b> {
    index: &'a SourceIndex,
    helpers: &'a std::collections::HashSet<Vec<String>>,
    body: &'a Body<'b>,
    local_uses: std::collections::HashMap<String, Vec<String>>,
    locked: usize,
    scan: &'a mut DispatchScan,
}

impl<'ast> syn::visit::Visit<'ast> for DispatchWalker<'_, '_> {
    fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
        if let Some(target) = self
            .index
            .resolve(&self.body.module, &self.local_uses, &node.path)
            .and_then(|path| self.index.function_at(path))
        {
            let locked = self.locked > 0;
            self.scan.function_references.push((target.clone(), locked));
            if is_scenario_function(&target) && !self.helpers.contains(&target) {
                self.scan.scenario_references.push(ScenarioReference {
                    target,
                    file: self.body.unit.file.clone(),
                    module: self.body.module.clone(),
                    context: self.body.context.clone(),
                    enclosing: self.body.enclosing.clone(),
                    locked,
                });
            }
        }
        syn::visit::visit_expr_path(self, node);
    }

    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        let calls_helper = matches!(node.func.as_ref(), syn::Expr::Path(path)
            if self.index.resolve_target(&self.body.module, &self.local_uses, &path.path)
                .is_some_and(|target| self.helpers.contains(&target)));
        if !calls_helper {
            syn::visit::visit_expr_call(self, node);
            return;
        }
        self.visit_expr(&node.func);
        for arg in &node.args {
            if matches!(arg, syn::Expr::Closure(_)) {
                self.locked += 1;
                self.visit_expr(arg);
                self.locked -= 1;
            } else {
                self.visit_expr(arg);
            }
        }
    }
}

/// Числительное для счёта, который эта проверка сверяет с прозой. Перечень короткий
/// намеренно: он покрывает правдоподобный размер поверхности, а не русский язык.
fn russian_numeral(count: usize) -> &'static str {
    match count {
        5 => "пять",
        6 => "шесть",
        7 => "семь",
        8 => "восемь",
        9 => "девять",
        10 => "десять",
        11 => "одиннадцать",
        12 => "двенадцать",
        other => panic!("числительного для {other} в проверке нет — допишите"),
    }
}

/// Вложенные шаги не берут блокировку повторно: замок рабочего каталога живёт на
/// границе адаптера, а сценарии зовут друг друга через входы без замка. Второй захват
/// изнутри дал бы «занято» самому себе.
///
/// Захват узнаётся по устройству, как у проверки границы: и `acquire_workspace_lock`, и
/// любой помощник замка, под каким бы именем он ни был.
#[test]
fn nested_orchestration_never_acquires_the_workspace_lock_inside_use_cases() {
    let relocks = relocks_in_the_scenario_layer(&SourceIndex::of_src());
    assert!(
        relocks.is_empty(),
        "a use case takes the workspace lock; nested steps run under the caller's lock:\n{}",
        relocks.join("\n")
    );

    // `test` строит перед прогоном тем же сценарием сборки, не выходя на границу адаптера.
    let run_tests = read("src/use_cases/run_tests/coordinator.rs");
    assert!(
        run_tests.contains("build_project::execute("),
        "run_tests must reuse the build use case directly, under the lock already held by the caller"
    );
}

/// Ссылки слоя сценариев на захват замка или его помощников. `workspace_lock` замок
/// реализует, `transport` — граница адаптера, где он берётся один раз за команду.
fn relocks_in_the_scenario_layer(index: &SourceIndex) -> Vec<String> {
    let lock = index.lock_api();
    let boundary = [
        path_of("crate::use_cases::workspace_lock"),
        path_of("crate::use_cases::transport"),
    ];
    let mut relocks = Vec::new();
    for body in production_bodies(index) {
        if !body.unit.in_scenario_layer() || boundary.contains(&body.module) {
            continue;
        }
        for target in lock.acquirers.iter().chain(&lock.helpers) {
            let mut finder = PathFinder {
                index,
                module: &body.module,
                local_uses: body.local_uses(index),
                target,
                found: false,
            };
            syn::visit::visit_block(&mut finder, body.block);
            if finder.found {
                relocks.push(format!(
                    "{} ({}): `{}`",
                    body.unit.file.display(),
                    body.context,
                    target.join("::")
                ));
            }
        }
    }
    relocks.sort();
    relocks
}

/// Проверка повторного захвата видит его и через помощника под любым именем.
#[test]
fn the_relock_guard_sees_a_lock_taken_through_a_helper() {
    let index = SourceIndex::from_sources(&[
        (
            "crate::use_cases::workspace_lock",
            "pub(crate) fn acquire_workspace_lock() -> Guard { Guard }",
        ),
        (
            "crate::use_cases::transport",
            "use crate::use_cases::workspace_lock::acquire_workspace_lock;\n\
             pub fn hold<T>(run: impl FnOnce() -> T) -> T { let _guard = acquire_workspace_lock(); run() }",
        ),
        (
            "crate::use_cases",
            "pub(crate) use self::transport::hold as relock;",
        ),
        (
            "crate::use_cases::nested",
            "use crate::use_cases::transport::hold;\n\
             pub fn execute() { hold(|| ()); }\n\
             pub fn renamed() { crate::use_cases::relock(|| ()); }\n\
             pub fn plain() {}",
        ),
    ]);

    assert_eq!(
        relocks_in_the_scenario_layer(&index),
        [
            "crate/use_cases/nested.rs (execute): `crate::use_cases::transport::hold`",
            "crate/use_cases/nested.rs (renamed): `crate::use_cases::transport::hold`",
        ]
    );
}

/// Лимит одновременных вызовов общий для обоих транспортов: семафор допуска создаётся
/// в одном месте, и оба конструктора — stdio и http — приходят к нему одной дорогой.
/// Второй `Semaphore::new` означал бы второй лимит, о котором конфиг не знает.
#[test]
fn mcp_admission_is_built_once_and_shared_by_both_transports() {
    let source = read("src/mcp/server.rs");
    let production = production_tokens(repo_path("src/mcp/server.rs").as_path());
    assert_eq!(
        production.matches("Semaphore::new(").count(),
        1,
        "admission must be built in exactly one place"
    );

    for constructor in ["fn stdio(", "fn http("] {
        let start = source
            .find(constructor)
            .unwrap_or_else(|| panic!("{constructor} constructor is missing"));
        let body = &source[start..];
        let end = body[constructor.len()..]
            .find("\n    pub fn ")
            .map(|offset| offset + constructor.len())
            .unwrap_or(body.len());
        let window = &body[..end];
        assert!(
            window.contains("with_port(") || window.contains("Self::new("),
            "{constructor} must build the server through the shared constructor"
        );
        assert!(
            !window.contains("Semaphore::new("),
            "{constructor} builds its own admission limit"
        );
    }
}

/// Слово исхода в подписи узла пишет presenter, а не рендерер.
///
/// Пока его писал каждый рендерер сам, подпись расходилась со знаком: `syntax` называл
/// проверку успешной, имея предупреждение среди подробностей, `test` обещал
/// предупреждения, не имея их. Оба расхождения нашли тесты, а не правило. Теперь слово
/// и знак берутся из одного значения (`NodeMark`), и рендереру незачем их произносить.
#[test]
fn a_renderer_never_spells_the_outcome_word_itself() {
    const OUTCOME_WORDS: &[&str] = &["completed successfully", "completed with warnings"];
    let renderers = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/cli/execute.rs");
    let text = fs::read_to_string(&renderers).expect("renderers are readable");
    for word in OUTCOME_WORDS {
        assert!(
            !text.contains(word),
            "{}: the outcome word `{word}` belongs to the presenter; name the subject and let it pick the word and the sign together",
            renderers.display()
        );
    }
}

/// Показ команды маскирует один владелец — `platform::secrets`. Пока его писала каждая
/// поверхность сама, превью запуска печатало `Pwd=***`, а отказ настоящего запуска той
/// же базы — `Pwd=s3cret`, и в stderr, и в журнал действий: правило, у которого четыре
/// исполнителя, — это четыре разных правила. Признак повтора — модуль, который строит
/// показ запрошенного процесса сам, не позвав владельца.
#[test]
fn a_process_command_is_shown_only_through_the_secrets_owner() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let owner = repo_path("src/platform/secrets.rs");

    for file in collect_rust_files(&repo_path("src")) {
        if file == owner {
            continue;
        }
        let production = production_tokens(&file);
        // `cmd:` — это поле показа у `ProcessError`; вместе с типом запроса оно и
        // означает, что модуль показывает argv запрошенного процесса. Поле `command`
        // журнала само по себе не признак: им называют и имя команды CLI.
        let requests_a_process = production.contains("ProcessRequest")
            || production.contains("InteractiveProcessRequest");
        if !(requests_a_process && production.contains("cmd:")) {
            continue;
        }

        assert!(
            production.contains("render_masked_command"),
            "{} shows a process command and must take the string from platform::secrets",
            file.strip_prefix(repo_root)
                .expect("relative path")
                .display()
        );
    }
}

/// Ключ `builder` снят решением `DEC.2026-09-14.BUILDER-KEY-IS-REMOVED`: конфиг с ним
/// не проходит валидацию. `tests/provider_matrix.rs` держит отказ со стороны рантайма,
/// а здесь — со стороны поставляемого навыка: `SKILL/` читают в чужих проектах, и
/// вернувшееся туда упоминание снова научило бы агентов писать конфиг, который раннер
/// отвергает. Единственный владелец выбора исполнителя — `providers.<операция>`.
#[test]
fn the_shipped_skill_never_names_the_removed_builder_key() {
    let skill_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("SKILL");
    let mut offenders = Vec::new();
    let mut stack = vec![skill_root.clone()];

    while let Some(path) = stack.pop() {
        for entry in fs::read_dir(&path).expect("read SKILL directory") {
            let entry = entry.expect("SKILL directory entry");
            let entry_path = entry.path();
            if entry_path.is_dir() {
                stack.push(entry_path);
                continue;
            }
            // Расширение не фильтруется: ключ может вернуться и в yaml рядом с навыком,
            // а «то же самое под другим именем» — это ровно то, что ловит эта проверка.
            if entry_path.extension().is_none() {
                continue;
            }
            let text = fs::read_to_string(&entry_path).expect("read SKILL file");
            for (index, line) in text.lines().enumerate() {
                if line.contains("builder") {
                    offenders.push(format!(
                        "{}:{}: {}",
                        entry_path
                            .strip_prefix(&skill_root)
                            .unwrap_or(&entry_path)
                            .display(),
                        index + 1,
                        line.trim()
                    ));
                }
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "SKILL/ must not name the removed `builder` key; use `providers.<operation>` instead:\n{}",
        offenders.join("\n")
    );
}

/// Маскирование секретов имеет одного владельца, и второй набор правил рядом с ним —
/// то, как эта дыра появилась в прошлый раз.
///
/// `run_tests` вёл собственный словарь флагов: знал `/P` и `/N` и не знал `/WSP`, `/UC`,
/// `/AccessToken`. Предыдущий guard его не видел, потому что смотрел только на модули,
/// показывающие argv процесса, а этот чистил чужую прозу. Здесь признак другой: файл,
/// который сам пишет регулярное выражение по секретному ключу, обязан звать владельца.
#[test]
fn secret_masking_rules_live_with_their_owner() {
    let owner = repo_path("src/platform/secrets.rs");
    // Ключи взяты из словаря владельца: их появление в регулярном выражении и означает
    // «здесь маскируют секрет».
    const SECRET_KEY_MARKERS: &[&str] = &["/P", "pwd=", "password=", "/WSP", "/UC"];
    let mut offenders = Vec::new();

    for file in collect_rust_files(&repo_path("src")) {
        if file == owner {
            continue;
        }
        // `production_tokens` убирает пробелы целиком, поэтому `Regex :: new` из текста
        // токенов снова читается как `Regex::new`. Искомые ключи пробелов не содержат.
        let production = production_tokens(&file);
        let writes_a_secret_regex = production.contains("Regex::new")
            && SECRET_KEY_MARKERS
                .iter()
                .any(|marker| production.contains(marker));
        if !writes_a_secret_regex {
            continue;
        }
        // Нужен вызов, а не упоминание: `platform::secrets` встречается и в прозе
        // комментария, поэтому признаком делегирования служит путь к элементу.
        if !production.contains("platform::secrets::") {
            offenders.push(file.display().to_string());
        }
    }

    assert!(
        offenders.is_empty(),
        "these modules mask secrets with their own rules instead of calling \
         platform::secrets, which is the single owner:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn the_loopback_question_is_answered_in_one_place() {
    // Корень проблемы: «петлевой ли адрес» решали сравнением подстрок, и `127.evil.com`
    // с `127.0.0.1@evil.com` проходили проверку. Владелец ответа один — `support::authority`,
    // потому что только разбор адреса знает про userinfo, скобки IPv6 и запись байтов.
    let owner = repo_path("src/support/authority.rs");
    // Первыми идут формы, которыми ошибка и была написана: сравнение по подстроке
    // и по префиксу. Без них защита стерегла бы только аккуратные написания — те,
    // которые и так безобидны, — и молчала бы ровно про тот дефект, чьё имя носит.
    const LOOPBACK_DECISION_MARKERS: &[&str] = &[
        "starts_with(\"127",
        "starts_with(\"::1",
        "contains(\"127",
        "contains(\"localhost",
        "ends_with(\"localhost",
        // Целиком закрытые литералы: `"127.0.0.1:3000"` из значения по умолчанию
        // ни под один из них не подходит, а плечо `match` и `Some("127")` — да.
        "\"127.\"",
        "\"127\"",
        "\"localhost\"",
        "\"::1\"",
        ".is_loopback()",
    ];
    let mut offenders = Vec::new();

    for file in collect_rust_files(&repo_path("src")) {
        if file == owner {
            continue;
        }
        let production = production_tokens(&file);
        let decides_about_loopback = LOOPBACK_DECISION_MARKERS
            .iter()
            .any(|marker| production.contains(marker));
        if !decides_about_loopback {
            continue;
        }
        if !production.contains("support::authority::") {
            offenders.push(file.display().to_string());
        }
    }

    assert!(
        offenders.is_empty(),
        "these modules decide what a loopback address is on their own instead of calling \
         support::authority, which is the single owner:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn a_host_port_record_is_read_in_one_place() {
    // Корень проблемы: запись `host:port` резали по последнему двоеточию в двух местах —
    // у SSH-шлюза автономного сервера и у `attach` агента, — и скобки IPv6 оставались в
    // хосте, который затем не разрешался. Владелец чтения один — `support::authority`:
    // только разбор адреса знает про скобки, порт, IDNA и запрещённые символы. Свои
    // правила поверх (порт обязателен) модули добавляют к его ответу, а не к строке.
    let owner = repo_path("src/support/authority.rs");
    // Маркеры порождаются разбором сниппетов: так они совпадают с тем, как `syn` печатает
    // токены, и не зависят от пробелов. Первые — формы, которыми дефект был написан.
    // Известная дыра: `parse::<SocketAddr>()` — тоже читатель адреса, но у него есть
    // законные места (`mcp.http.bind_address` — числовой адрес привязки), и маркером он
    // станет вместе со своим allowlist.
    let markers: Vec<(String, &str)> = [
        "rsplit_once(':')",
        "rsplit_once(\":\")",
        "rsplitn(2, ':')",
        "rsplitn(2, \":\")",
        "rfind(':')",
        "rfind(\":\")",
        "split_once(':')",
        "split_once(\":\")",
    ]
    .into_iter()
    .map(|call| (cut_marker(call), call))
    .collect();
    // Строки `ключ: значение` режут по первому двоеточию законно: это не адреса. Список
    // может только сокращаться.
    const KEY_VALUE_LINE_READERS: &[&str] = &[
        "src/platform/extension_inventory.rs",
        "src/support/edt_project.rs",
    ];
    let mut offenders = Vec::new();

    for file in collect_rust_files(&repo_path("src")) {
        if file == owner {
            continue;
        }
        let relative = file
            .strip_prefix(repo_path(""))
            .expect("inside the repository")
            .to_string_lossy()
            .replace('\\', "/");
        let source = without_doc_comments(&production_source(&file));
        for (marker, call) in &markers {
            if !source.contains(marker.as_str()) {
                continue;
            }
            if call.starts_with("split_once") && KEY_VALUE_LINE_READERS.contains(&relative.as_str())
            {
                continue;
            }
            offenders.push(format!("{relative}: {call}"));
        }
    }

    assert!(
        offenders.is_empty(),
        "these modules cut a host:port record by a colon on their own instead of calling \
         support::authority::host_and_port_of_authority, which is the single owner:\n{}",
        offenders.join("\n")
    );

    // Запись allowlist, за которой больше нет `split_once` по двоеточию, устарела: список
    // может только сокращаться, и стареть молча ему нельзя.
    let split_once_markers: Vec<&String> = markers
        .iter()
        .filter(|(_, call)| call.starts_with("split_once"))
        .map(|(marker, _)| marker)
        .collect();
    let stale: Vec<&str> = KEY_VALUE_LINE_READERS
        .iter()
        .copied()
        .filter(|relative| {
            let source = without_doc_comments(&production_source(&repo_path(relative)));
            !split_once_markers
                .iter()
                .any(|marker| source.contains(marker.as_str()))
        })
        .collect();
    assert!(
        stale.is_empty(),
        "stale KEY_VALUE_LINE_READERS entries — the files no longer split a line by a colon, \
         remove them from the allowlist:\n{}",
        stale.join("\n")
    );
}

/// Токены вызова `.<call>` в том виде, в каком их печатает `syn`, без приёмника.
fn cut_marker(call: &str) -> String {
    let expression: syn::Expr =
        syn::parse_str(&format!("v.{call}")).unwrap_or_else(|_| panic!("parse {call}"));
    let rendered = quote::ToTokens::to_token_stream(&expression).to_string();
    rendered
        .strip_prefix("v . ")
        .unwrap_or_else(|| panic!("receiver in {rendered}"))
        .to_owned()
}

/// Убирает `# [doc = "..."]` из исходного вида токенов, оставляя сам код. Литерал
/// документации читается до неэкранированной закрывающей кавычки: `]` внутри прозы
/// (`[v6]:port`) — не конец атрибута.
fn without_doc_comments(source: &str) -> String {
    const OPENING: &str = "# [doc = ";
    let mut kept = String::with_capacity(source.len());
    let mut rest = source;

    while let Some(at) = rest.find(OPENING) {
        kept.push_str(&rest[..at]);
        let after = &rest[at + OPENING.len()..];
        let Some(literal) = string_literal_len(after) else {
            return kept;
        };
        match after[literal..].find(']') {
            Some(close) => rest = &after[literal + close + 1..],
            None => return kept,
        }
    }
    kept.push_str(rest);
    kept
}

/// Длина строкового литерала в начале `text`, с кавычками, если он там стоит.
fn string_literal_len(text: &str) -> Option<usize> {
    let mut chars = text.char_indices();
    let Some((_, '"')) = chars.next() else {
        return None;
    };
    let mut escaped = false;
    for (index, ch) in chars {
        match ch {
            '\\' if !escaped => escaped = true,
            '"' if !escaped => return Some(index + 1),
            _ => escaped = false,
        }
    }
    None
}

#[test]
fn the_http_listener_never_hands_out_a_cross_origin_permission() {
    // Проверка `Origin` у слушателя MCP мягкая нарочно: она пускает любой петлевой
    // порт и все имена из `mcp.http.allowed_hosts`. Держится это на том, что
    // межисточниковый запрос браузер гасит сам, не получив разрешения. Стоит выдать
    // его — и мягкость превратится в дыру: каждое имя из списка станет читаемым из
    // чужого источника. Поэтому разрешения нет нигде.
    //
    // Приметы сравниваются в нижнем регистре: `HeaderName` приводит имя к нему сам,
    // поэтому написание в исходнике роли не играет, а точное совпадение по регистру
    // пропустило бы рабочую выдачу.
    const CROSS_ORIGIN_GRANTS: &[&str] = &[
        "access-control-allow-origin",
        "access_control_allow_origin",
        "corslayer",
        "tower_http::cors",
    ];
    let mut offenders = Vec::new();

    for file in collect_rust_files(&repo_path("src")) {
        // Документация про правило — не нарушение правила: `///` попадает в токены
        // как `#[doc="..."]`, и без этого описать запрет рядом с кодом было бы нельзя.
        let production = without_doc_attributes(&production_tokens(&file)).to_ascii_lowercase();
        if CROSS_ORIGIN_GRANTS
            .iter()
            .any(|grant| production.contains(grant))
        {
            offenders.push(file.display().to_string());
        }
    }

    let manifest = read("Cargo.toml").to_ascii_lowercase();
    assert!(
        !manifest.contains("\"cors\"") && !manifest.contains("'cors'"),
        "Cargo.toml enables a CORS layer; the MCP Origin rule assumes none is ever built"
    );
    assert!(
        offenders.is_empty(),
        "these modules hand out a cross-origin permission, which turns the deliberately \
         lenient MCP Origin rule into a readable cross-origin surface:\n{}",
        offenders.join("\n")
    );
}

/// Убирает из токенов содержимое `#[doc="..."]`, оставляя сам код.
fn without_doc_attributes(tokens: &str) -> String {
    const OPENING: &str = "#[doc=";
    let mut kept = String::with_capacity(tokens.len());
    let mut rest = tokens;

    while let Some(at) = rest.find(OPENING) {
        kept.push_str(&rest[..at]);
        let after = &rest[at + OPENING.len()..];
        match after.find(']') {
            Some(close) => rest = &after[close + 1..],
            None => return kept,
        }
    }
    kept.push_str(rest);
    kept
}

#[test]
fn the_http_listener_is_never_served_without_its_host_check() {
    // Проверка имени хоста — слой поверх маршрутизатора, и снять её можно одной
    // строкой: сборка останется зелёной везде, кроме `tests/mcp_http.rs`, а тот
    // объявлен `#![cfg(unix)]` и до Windows не доезжает. Поэтому саму проводку
    // держит примета: тот, кто поднимает слушатель, обязан навесить слой.
    let window = free_function_tokens(repo_path("src/mcp/server.rs").as_path(), "serve_http");

    assert!(
        !window.is_empty(),
        "serve_http is gone from src/mcp/server.rs; this guard names the wrong function"
    );
    for required in [
        "axum::serve",
        "from_fn_with_state",
        "refuse_a_request_that_names_another_host",
    ] {
        assert!(
            window.contains(required),
            "serve_http no longer wires the host check ({required} is missing): a listener \
             built without it answers any name, which is the DNS-rebinding hole itself"
        );
    }
}

#[test]
fn a_command_carries_no_deadline_anywhere_it_could_be_put_back() {
    // Корень проблемы: общий срок на команду не защищал базу, а ломал её — истёкший срок
    // означал, что загрузка одного набора исходников уже зафиксирована, а следующий отказан
    // на безопасной точке, то есть конфигурация обновлена наполовину
    // (DEC.2026-09-20.A-COMMAND-HAS-NO-DEADLINE).
    //
    // Владелец ответа один — `ExecutionContext`: у него нет поля срока и нет способа его
    // поставить, поэтому вернуть срок можно только заведя это поле заново. Страж стоит на
    // самих именах, а не на поведении: тест поведения проходит и без срока, и со сроком,
    // который никто не выставил, и потому регресс пропустит.
    const BANNED_IN_CONTEXT: &[&str] = &["deadline", "remaining_budget", "TimedOut"];

    let context =
        without_doc_attributes(&production_tokens(&repo_path("src/use_cases/context.rs")));
    let offenders: Vec<&str> = BANNED_IN_CONTEXT
        .iter()
        .copied()
        .filter(|marker| context.contains(marker))
        .collect();
    assert!(
        offenders.is_empty(),
        "src/use_cases/context.rs names {offenders:?}: a command budget is back on the execution \
         context. A bound belongs to the step that declares it, not above it."
    );

    // Срок уже однажды воскресал под другим именем: остаток допускного бюджета укорачивал
    // предел шага EDT в MCP. Допускной срок обязан кончаться вместе с допуском.
    let server = without_doc_attributes(&production_tokens(&repo_path("src/mcp/server.rs")));
    assert!(
        !server.contains("remaining_timeout"),
        "src/mcp/server.rs computes a remainder of the admission budget: an admitted call must \
         run to its terminal outcome, and a step cap must not be shortened by the queue wait."
    );
}

#[test]
fn a_reply_is_checked_against_its_form_through_one_reader() {
    // Корень проблемы: тесты сверяли `data` с формами своими копиями чтения схемы, и копии
    // расходились с общей — одна брала первую из общих форм, другая жёстко записанный путь,
    // и ни одна не знала, что общая форма отказа проходит за любую. Владелец один —
    // `tests/support/command_data.rs`; схему конверта и примеры правил сверяют свои
    // проверки. Страж стоит на имени крейта: без него схему не прочитать.
    const SCHEMA_READERS: &[&str] = &[
        "tests/support/command_data.rs",
        "tests/contract_envelope.rs",
        "tests/arch_rules.rs",
        // Сам страж называет искомое имя.
        "tests/architecture_guardrails.rs",
    ];

    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offenders = Vec::new();
    for file in collect_rust_files(&repo_path("tests")) {
        let relative = file.strip_prefix(repo_root).expect("relative path");
        if SCHEMA_READERS
            .iter()
            .any(|reader| relative == Path::new(reader))
        {
            continue;
        }
        if fs::read_to_string(&file)
            .expect("read test source")
            .contains("jsonschema")
        {
            offenders.push(relative.display().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "these tests read a data form themselves; check a reply through \
         tests/support/command_data.rs instead:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn every_staged_publication_rechecks_its_target_first() {
    // Корень проблемы: цель сверялась при разрешении, а публикация шла после работы
    // исполнителя; у `make` путь, который за это время стал указывать в другое место,
    // публиковался молча. Перепроверку ставит каждое место публикации само — единого
    // владельца у неё пока нет (#300), — поэтому страж смотрит на каждое место: в теле
    // сценария вызов публикации через промежуточную копию идёт после перепроверки цели, а
    // перепроверка — после подготовки копии и после проверки исхода исполнителя, если та
    // стоит между ними. Страж держится на именах перепроверок, а их тела держат
    // поведенческие тесты правила `INV.USE-CASES.A-TARGET-IS-RECHECKED-BEFORE-PUBLICATION`.
    const RECHECKS: &[&str] = &[
        "refusal_before_publication(",
        "validate_platform_target(",
        "validate_publish_target(",
        "revalidate_before_publish(",
    ];
    // Проверки исхода исполнителя: перепроверка, поставленная раньше них, сверяла бы цель до
    // работы исполнителя.
    const EXECUTOR_OUTCOME: &[&str] = &[
        "ensure_platform_success(",
        "ensure_import_success(",
        "validate_platform_success(",
    ];
    // Места публикации названы: вызов, ушедший туда, где страж его не видит, не пройдёт
    // молча, а новое место попадёт в перечень осознанно.
    const SITES: &[&str] = &[
        "crate::use_cases::artifacts::agent::run_agent_export",
        "crate::use_cases::artifacts::agent::run_external_agent_export",
        "crate::use_cases::artifacts::run_designer_export",
        "crate::use_cases::artifacts::run_external_designer_export",
        "crate::use_cases::dump_config::agent::publish_full",
        "crate::use_cases::dump_config::finalize_edt_dump",
        "crate::use_cases::dump_config::run_full_dump_designer",
        "crate::use_cases::dump_config::run_full_dump_ibcmd",
        "crate::use_cases::infobase_export::execute_configuration_export",
        "crate::use_cases::infobase_export::execute_infobase_snapshot",
    ];

    let index = SourceIndex::of_src();
    let owner = path_of("crate::use_cases::staged_publication");
    let (publish, prepare) = staged_publication_methods(&index, &owner);
    let scenarios = path_of("crate::use_cases");
    let mut sites = Vec::new();
    let mut offenders = Vec::new();
    for body in production_bodies(&index) {
        if !body.module.starts_with(&scenarios) || body.module == owner {
            continue;
        }
        let tokens = normalize_tokens(body.block);
        let site = format!("{}::{}", body.module.join("::"), body.context);
        for at in publish
            .iter()
            .flat_map(|needle| tokens.match_indices(needle.as_str()).map(|(at, _)| at))
        {
            sites.push(site.clone());
            let prepared = prepare
                .iter()
                .filter_map(|needle| tokens[..at].rfind(needle.as_str()))
                .max()
                .unwrap_or(0);
            let executed = EXECUTOR_OUTCOME
                .iter()
                .filter_map(|needle| tokens[prepared..at].rfind(needle).map(|i| prepared + i))
                .max()
                .unwrap_or(prepared);
            if !RECHECKS
                .iter()
                .any(|needle| tokens[executed..at].contains(needle))
            {
                offenders.push(site.clone());
            }
        }
    }
    sites.sort();
    assert!(
        offenders.is_empty(),
        "these scenarios publish without re-checking the target after the executor ran:\n{}",
        offenders.join("\n")
    );
    assert_eq!(
        sites, SITES,
        "the staged publications changed; name each one here after it re-checks its target"
    );
}

/// Вызовы публикации и подготовки у владельца промежуточной копии — в виде, в котором их
/// ищут в теле сценария: методом (`.publish_dir(`) и путём (`::publish_dir(`). Имена берутся
/// из самого `impl StagedPublication`, поэтому новый метод попадёт под страж сам.
fn staged_publication_methods(index: &SourceIndex, owner: &[String]) -> (Vec<String>, Vec<String>) {
    let unit = index
        .units
        .iter()
        .find(|unit| unit.module == owner)
        .expect("the staged publication owner is indexed");
    let mut publish = Vec::new();
    let mut prepare = Vec::new();
    for item in &unit.syntax.items {
        let syn::Item::Impl(item_impl) = item else {
            continue;
        };
        let is_owner = matches!(
            item_impl.self_ty.as_ref(),
            syn::Type::Path(type_path)
                if type_path.path.segments.last().is_some_and(|segment| segment.ident == "StagedPublication")
        );
        if !is_owner || item_impl.trait_.is_some() {
            continue;
        }
        for impl_item in &item_impl.items {
            let syn::ImplItem::Fn(method) = impl_item else {
                continue;
            };
            if matches!(method.vis, syn::Visibility::Inherited) {
                continue;
            }
            let name = method.sig.ident.to_string();
            let needles = [format!(".{name}("), format!("::{name}(")];
            if name.starts_with("publish") {
                publish.extend(needles);
            } else if name.starts_with("prepare") {
                prepare.extend(needles);
            }
        }
    }
    assert!(
        !publish.is_empty() && !prepare.is_empty(),
        "StagedPublication no longer names its publish and prepare methods"
    );
    (publish, prepare)
}

#[test]
fn change_detection_has_no_background_watcher() {
    // Изменения ищет та команда, которой нужен ответ; фонового наблюдателя нет. Наблюдатель —
    // это поток рядом с командой или крейт слежения за файловой системой: слой анализа
    // потоков не заводит, а раннер такого крейта не тянет.
    const WATCHER_CRATES: &[&str] = &["notify", "notify-debouncer-mini", "hotwatch"];
    const BACKGROUND_WORK: &[&str] = &["spawn", "thread::"];

    let manifest = read("Cargo.toml");
    let watchers: Vec<&str> = manifest
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(name, _)| name.trim())
        .filter(|name| WATCHER_CRATES.contains(name))
        .collect();
    assert!(
        watchers.is_empty(),
        "Cargo.toml depends on a file-system watcher {watchers:?}: change detection runs when \
         a command needs the answer, not in the background"
    );

    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offenders = Vec::new();
    for file in collect_rust_files(&repo_path("src/change_detection")) {
        let relative = file.strip_prefix(repo_root).expect("relative path");
        let production = without_doc_attributes(&production_tokens(&file));
        for marker in BACKGROUND_WORK {
            if production.contains(marker) {
                offenders.push(format!("{} names `{marker}`", relative.display()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "change detection starts background work; it must run only when a command asks:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn change_detection_never_reads_the_executor_choice() {
    // Анализ изменений отвечает, нужен ли шаг; чем его выполнить, решает выбор исполнителя.
    // Прочитай слой анализа этот выбор — и ответ «что делать» начал бы зависеть от того,
    // кто делает: смена исполнителя меняла бы решение о загрузке. Страж стоит на именах:
    // слой не называет ни ключей выбора, ни матрицы исполнителей, ни адаптеров платформы.
    // Конфигурацию слой читать вправе — наборы, формат и `workPath` берутся из неё.
    const EXECUTOR_CHOICE: &[&str] = &["provider", "Provider", "capability", "platform::"];

    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let files = collect_rust_files(&repo_path("src/change_detection"));
    assert!(
        !files.is_empty(),
        "the change-detection layer is not where it was"
    );
    let mut offenders = Vec::new();
    for file in files {
        let relative = file.strip_prefix(repo_root).expect("relative path");
        let production = without_doc_attributes(&production_tokens(&file));
        for name in EXECUTOR_CHOICE {
            if production.contains(name) {
                offenders.push(format!("{} names `{name}`", relative.display()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "change detection reads the executor choice; whether a step is needed must not depend \
         on who performs it:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn every_top_level_module_is_a_block_of_the_module_map() {
    // Корень проблемы: карта модулей жила в двух файлах, на маршруте агента лежал один, и ни
    // один не был привязан к изменениям кода — три модуля так и не попали ни в один. Владелец
    // карты один — таблица 5.1 раздела 5 arc42. Модуль без строки в ней валит этот страж, где
    // бы вторую карту ни завели; ссылка в прозе рядом строки не заменяет.
    let main = parse_rust_file(&repo_path("src/main.rs"));
    let modules: Vec<String> = main
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Mod(module) if !has_cfg_test(&module.attrs) => {
                Some(module.ident.to_string())
            }
            _ => None,
        })
        .collect();
    assert!(
        !modules.is_empty(),
        "src/main.rs declares no modules: the module list was not read"
    );
    let map = read("spec/arc42/05-building-block-view.md");
    // Строку модуля называет её первая ячейка: ссылка в чужой строке таблицы или в прозе
    // строку не заменяет.
    let first_cells: Vec<&str> = extract_between(&map, "### 5.1", "### 5.2")
        .lines()
        .filter_map(|line| line.trim().strip_prefix('|'))
        .filter_map(|row| row.split('|').next())
        .collect();
    let missing: Vec<&str> = modules
        .iter()
        .map(String::as_str)
        .filter(|name| {
            let as_dir = format!("](../../src/{name}/)");
            let as_file = format!("](../../src/{name}.rs)");
            !first_cells
                .iter()
                .any(|cell| cell.contains(&as_dir) || cell.contains(&as_file))
        })
        .collect();
    assert!(
        missing.is_empty(),
        "table 5.1 of spec/arc42/05-building-block-view.md has no row linking {missing:?}: the \
         module map names every top-level module of src/main.rs"
    );
}

/// Документы маршрута агента, чьи ссылки сторожит `every_link_on_the_agent_route_resolves`;
/// к ним — каждый файл `spec/arc42/`.
const AGENT_ROUTE_DOCUMENTS: &[&str] = &[
    "AGENTS.md",
    "AI_DEV.md",
    "README.md",
    "docs/README.md",
    "spec/README.md",
    "spec/rules/README.md",
];

#[test]
fn every_link_on_the_agent_route_resolves() {
    // Описание устройства называет модули и файлы ссылками, маршрут агента — документы.
    // Переименованный файл делает адрес ложным молча, и агент, пришедший по нему, остаётся
    // без ответа. Страж привязывает эти тексты к изменениям дерева. Регистр сверяется точно:
    // macOS и Windows его прощают, Linux и GitHub — нет.
    let root = repo_path("");
    let arc42 = repo_path("spec/arc42");
    let mut documents: Vec<PathBuf> = AGENT_ROUTE_DOCUMENTS
        .iter()
        .map(|relative| repo_path(relative))
        .collect();
    documents.extend(
        fs::read_dir(&arc42)
            .unwrap_or_else(|error| panic!("{}: {error}", arc42.display()))
            .map(|entry| entry.expect("directory entry").path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "md")),
    );
    documents.sort();

    let mut seen = 0usize;
    let mut broken = Vec::new();
    for document in &documents {
        let text = fs::read_to_string(document)
            .unwrap_or_else(|error| panic!("{}: {error}", document.display()));
        let base = document
            .parent()
            .expect("a document has a parent directory");
        for target in relative_link_targets(&text) {
            seen += 1;
            if !resolves_with_exact_case(&root, base, &target) {
                broken.push(format!("{}: {target}", repo_relative(document)));
            }
        }
    }
    // Разбор, который не нашёл ни одной ссылки, прошёл бы зелёным и ничего не сторожил.
    assert!(
        seen >= 50,
        "only {seen} relative links found on the agent route: the link reader is broken"
    );
    assert!(
        broken.is_empty(),
        "links on the agent route point at nothing (case is compared exactly):\n{}",
        broken.join("\n")
    );
}

/// Путь от корня репозитория, всегда через косую черту: `Path::display()` на Windows дал бы
/// обратную.
fn repo_relative(path: &Path) -> String {
    path.strip_prefix(repo_path(""))
        .unwrap_or(path)
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Встроенная ссылка `[текст](адрес)` или `[текст](<адрес>)`, с заголовком или без. Голый
/// адрес может нести парные скобки: `guide(v2).md`.
static INLINE_LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"\]\((?:<([^>]*)>|((?:[^()\s]|\([^()\s]*\))+))(?:\s+(?:"[^"]*"|'[^']*'|\([^)]*\)))?\)"#,
    )
    .expect("regex")
});
/// Сноска `[метка]: адрес` или `[метка]: <адрес>`; `[^метка]:` — примечание, а не ссылка.
static REFERENCE_LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^ {0,3}\[[^\]^][^\]]*\]:\s*(?:<([^>]*)>|(\S+))").expect("regex")
});
/// Строка, с которой начинается новый блок: пункт списка или строка таблицы. Код в строке
/// через границу блока не переходит.
static BLOCK_START: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?:[-*+]\s|\d+[.)]\s|\|)").expect("regex"));

/// Относительные адреса ссылок — встроенных и сносок — вне огороженных блоков и вне кода в
/// строке. Внешние адреса и якоря своей страницы сторожа не касаются.
fn relative_link_targets(text: &str) -> Vec<String> {
    // Огороженный блок выпадает целиком, а его строки остаются пустыми, чтобы соседние абзацы
    // не склеились.
    let mut prose = String::with_capacity(text.len());
    let mut fence: Option<(char, usize)> = None;
    for line in text.lines() {
        match (fence, fence_marker(line)) {
            (None, Some(opened)) => fence = Some(opened),
            // Ограду закрывает тот же знак, серия не короче открывшей и ничего после неё.
            (Some((open, length)), Some((close, run)))
                if close == open && run >= length && closes_alone(line, run) =>
            {
                fence = None;
            }
            // Строка внутри ограды, в том числе с чужим или коротким знаком.
            (Some(_), _) => {}
            (None, None) => prose.push_str(line),
        }
        prose.push('\n');
    }
    let mut targets = Vec::new();
    for block in blocks(&prose) {
        let paragraph = without_code_spans(&block);
        let inline = INLINE_LINK
            .captures_iter(&paragraph)
            .filter_map(|capture| capture.get(1).or_else(|| capture.get(2)));
        let reference = REFERENCE_LINK
            .captures_iter(&paragraph)
            .filter_map(|capture| capture.get(1).or_else(|| capture.get(2)));
        for found in inline.chain(reference) {
            let path = found.as_str().split('#').next().unwrap_or_default();
            if !path.is_empty() && !path.contains(':') {
                targets.push(path.to_owned());
            }
        }
    }
    targets
}

/// Знак ограды CommonMark: не больше трёх пробелов отступа, затем серия не короче трёх
/// одинаковых знаков — обратных кавычек или тильд. Четыре пробела отступа делают строку
/// кодом с отступом, а не оградой.
fn fence_marker(line: &str) -> Option<(char, usize)> {
    let rest = line.trim_start_matches(' ');
    if line.len() - rest.len() > 3 {
        return None;
    }
    let mark = rest.chars().next().filter(|ch| *ch == '`' || *ch == '~')?;
    let run = rest.chars().take_while(|ch| *ch == mark).count();
    (run >= 3).then_some((mark, run))
}

/// Строка закрытия ограды: после серии знаков — только пробелы.
fn closes_alone(line: &str, run: usize) -> bool {
    line.trim_start_matches(' ')[run..].trim().is_empty()
}

/// Блоки текста, в пределах которых живёт код в строке: абзацы между пустыми строками, а
/// внутри них — пункты списка и строки таблицы.
fn blocks(prose: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current = String::new();
    for line in prose.lines() {
        let starts_block = line.trim().is_empty() || BLOCK_START.is_match(line);
        if starts_block && !current.is_empty() {
            blocks.push(std::mem::take(&mut current));
        }
        current.push_str(line);
        current.push('\n');
    }
    if !current.is_empty() {
        blocks.push(current);
    }
    blocks
}

/// Блок без кода в строке — так, как его читает CommonMark: код открывает серия обратных
/// кавычек и закрывает серия той же длины в том же блоке, а серия без пары и кавычка после
/// обратной косой черты — просто знаки. Поэтому лишняя кавычка не прячет ни одной ссылки, а
/// текст ссылки в кавычках пустеет, но её адрес остаётся. Переводы строк из кода
/// сохраняются: сноска должна остаться в начале своей строки.
fn without_code_spans(paragraph: &str) -> String {
    let mut kept = String::with_capacity(paragraph.len());
    let mut rest = paragraph;
    while let Some(open) = rest.find('`') {
        let slashes = rest[..open]
            .bytes()
            .rev()
            .take_while(|&byte| byte == b'\\')
            .count();
        if slashes % 2 == 1 {
            kept.push_str(&rest[..=open]);
            rest = &rest[open + 1..];
            continue;
        }
        kept.push_str(&rest[..open]);
        let run = backtick_run(&rest[open..]);
        let body = &rest[open + run..];
        match closing_run(body, run) {
            Some(close) => {
                kept.extend(body[..close].chars().filter(|&ch| ch == '\n'));
                rest = &body[close + run..];
            }
            None => {
                kept.push_str(&rest[open..open + run]);
                rest = body;
            }
        }
    }
    kept.push_str(rest);
    kept
}

/// Длина серии обратных кавычек в начале строки, в байтах: знак однобайтовый.
fn backtick_run(text: &str) -> usize {
    text.bytes().take_while(|&byte| byte == b'`').count()
}

/// Начало первой серии ровно из `run` обратных кавычек.
fn closing_run(text: &str, run: usize) -> Option<usize> {
    let mut offset = 0;
    while let Some(found) = text[offset..].find('`') {
        let start = offset + found;
        let length = backtick_run(&text[start..]);
        if length == run {
            return Some(start);
        }
        offset = start + length;
    }
    None
}

#[test]
fn the_link_reader_sees_what_markdown_renders() {
    // Страж ссылок стоит на этом разборе: пропущенная им ссылка не проверяется вовсе, и
    // страж остаётся зелёным. Здесь — случаи, на которых разбор уже ошибался.
    let text = "\
[a](one.md) и [`b`](two.md \"заголовок\") и [c](<three four.md>)

```
[x](fenced.md)
```

~~~
[y](tilde.md)
~~~

Нажмите клавишу ` — [d](five.md)

``код с ` внутри [z](code.md)`` и затем [e](six.md)

- пункт с лишней ` кавычкой
- следующий пункт: [g](nine.md 'заголовок')

| ` | ячейка |
| --- | [h](ten.md (заголовок)) |

Экранированная \\` кавычка и [i](eleven.md)

[j](guide(v2).md)

[angle ref]: <twelve thirteen.md>

    ```
[k](fourteen.md)

````
```
[w](inside-long-fence.md)
````

[ref]: seven.md
[^note]: примечание, а не ссылка

[ext](https://example.com) [якорь](#here) [f](eight.md#part)
";
    let mut found = relative_link_targets(text);
    found.sort();
    assert_eq!(
        found,
        [
            "eight.md",
            "eleven.md",
            "five.md",
            "fourteen.md",
            "guide(v2).md",
            "nine.md",
            "one.md",
            "seven.md",
            "six.md",
            "ten.md",
            "three four.md",
            "twelve thirteen.md",
            "two.md"
        ]
    );
}

/// Путь существует с точностью до регистра: `..` сворачивается по тексту, а каждый
/// компонент сверяется с перечнем своего каталога, а не с ответом файловой системы.
fn resolves_with_exact_case(root: &Path, base: &Path, target: &str) -> bool {
    let Ok(relative_base) = base.strip_prefix(root) else {
        return false;
    };
    let mut parts: Vec<OsString> = Vec::new();
    for component in relative_base.join(target).components() {
        match component {
            Component::Normal(part) => parts.push(part.to_os_string()),
            Component::ParentDir => {
                if parts.pop().is_none() {
                    return false;
                }
            }
            Component::CurDir => {}
            Component::RootDir | Component::Prefix(_) => return false,
        }
    }
    let mut current = root.to_path_buf();
    for part in parts {
        let Ok(entries) = fs::read_dir(&current) else {
            return false;
        };
        if !entries.flatten().any(|entry| entry.file_name() == part) {
            return false;
        }
        current.push(part);
    }
    true
}

/// Что тело кода делает с признаком `provider_dispatched` и с отметкой работы исполнителя.
#[derive(Default)]
struct DispatchUse {
    /// Значения признака в литералах структур и в присваиваниях, которыми он мог бы сказать
    /// о работе: не `false`, не `None`, не `Some(false)` и не копия чужого признака.
    decided: Vec<String>,
    /// Копии чужого признака: `provider_dispatched: other.provider_dispatched`.
    copies: usize,
    /// Изменяемые заимствования признака и составные присваивания (`|=`, `&=`, …).
    writes: usize,
    /// Макросы с признаком, чьё тело не разбирается как список выражений: заглянуть в него
    /// страж не может.
    opaque_macros: usize,
    marks: usize,
    stamp_work: usize,
    for_command: usize,
    /// Шаг, объявленный не работой команды: `work: None` или `None` последним аргументом
    /// конструктора политики процесса, запроса к сессии EDT или `spawn_managed`.
    without_work: usize,
    policies: usize,
    contexts: usize,
    stamps: usize,
    returns: usize,
    tries: usize,
}

impl DispatchUse {
    fn of(block: &syn::Block) -> Self {
        let mut found = Self::default();
        syn::visit::Visit::visit_block(&mut found, block);
        found
    }

    fn note_value(&mut self, value: &syn::Expr) {
        if is_dispatch_field(value) {
            self.copies += 1;
        } else if !is_honest_dispatch(value) {
            self.decided.push(normalize_tokens(value));
        }
    }
}

fn is_dispatch_field(expr: &syn::Expr) -> bool {
    matches!(expr, syn::Expr::Field(field)
        if matches!(&field.member, syn::Member::Named(name) if name == "provider_dispatched"))
}

fn is_literal_false(expr: &syn::Expr) -> bool {
    matches!(expr, syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Bool(value), .. }) if !value.value)
}

fn is_none(expr: &syn::Expr) -> bool {
    matches!(expr, syn::Expr::Path(path) if path.path.is_ident("None"))
}

/// Значение, которым признак о работе сказать не может: `false`, `None`, `Some(false)`.
fn is_honest_dispatch(expr: &syn::Expr) -> bool {
    match expr {
        syn::Expr::Call(call) => {
            matches!(call.func.as_ref(), syn::Expr::Path(path) if path.path.is_ident("Some"))
                && call.args.len() == 1
                && call.args.first().is_some_and(is_literal_false)
        }
        other => is_literal_false(other) || is_none(other),
    }
}

fn is_stamp_call(call: &syn::ExprCall) -> bool {
    matches!(call.func.as_ref(), syn::Expr::Path(path)
        if path.path.segments.last().is_some_and(|segment| segment.ident == "stamp_dispatch"))
}

/// Хвост, на котором штамп стоит на всяком исходе: сам вызов штампа или `.map(|x| штамп)`
/// над исходом, чья ошибка — транспортная — формы не несёт.
fn is_stamped_tail(expr: &syn::Expr) -> bool {
    match expr {
        syn::Expr::Call(call) => is_stamp_call(call),
        syn::Expr::MethodCall(method) if method.method == "map" && method.args.len() == 1 => {
            matches!(method.args.first(), Some(syn::Expr::Closure(closure))
                if matches!(closure.body.as_ref(), syn::Expr::Call(call) if is_stamp_call(call)))
        }
        _ => false,
    }
}

impl<'ast> syn::visit::Visit<'ast> for DispatchUse {
    fn visit_field_value(&mut self, node: &'ast syn::FieldValue) {
        if let syn::Member::Named(name) = &node.member {
            if name == "provider_dispatched" {
                if node.colon_token.is_none() {
                    // `R { provider_dispatched }`: значение пришло из переменной.
                    self.decided.push(name.to_string());
                } else {
                    self.note_value(&node.expr);
                }
            }
            if name == "work" && is_none(&node.expr) {
                self.without_work += 1;
            }
        }
        syn::visit::visit_field_value(self, node);
    }

    fn visit_expr_assign(&mut self, node: &'ast syn::ExprAssign) {
        if is_dispatch_field(&node.left) {
            self.note_value(&node.right);
        }
        syn::visit::visit_expr_assign(self, node);
    }

    fn visit_expr_binary(&mut self, node: &'ast syn::ExprBinary) {
        let compound = matches!(
            node.op,
            syn::BinOp::AddAssign(_)
                | syn::BinOp::SubAssign(_)
                | syn::BinOp::MulAssign(_)
                | syn::BinOp::DivAssign(_)
                | syn::BinOp::RemAssign(_)
                | syn::BinOp::BitXorAssign(_)
                | syn::BinOp::BitAndAssign(_)
                | syn::BinOp::BitOrAssign(_)
                | syn::BinOp::ShlAssign(_)
                | syn::BinOp::ShrAssign(_)
        );
        if compound && is_dispatch_field(&node.left) {
            self.writes += 1;
        }
        syn::visit::visit_expr_binary(self, node);
    }

    fn visit_expr_reference(&mut self, node: &'ast syn::ExprReference) {
        if node.mutability.is_some() && is_dispatch_field(&node.expr) {
            self.writes += 1;
        }
        syn::visit::visit_expr_reference(self, node);
    }

    /// Тело макроса, похожее на список выражений (`vec![…]`, `format!(…)`, `debug!(…)`),
    /// проверяется как код; иначе упоминание признака в нём — уже нарушение.
    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        match node.parse_body_with(
            syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated,
        ) {
            Ok(arguments) => {
                for argument in &arguments {
                    syn::visit::Visit::visit_expr(self, argument);
                }
            }
            Err(_) if mentions_dispatch(&node.tokens) => self.opaque_macros += 1,
            Err(_) => {}
        }
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        if node.method == "mark_work_given" {
            self.marks += 1;
        }
        if node.method == "stamp_work" {
            self.stamp_work += 1;
        }
        if node.method == "spawn_managed" && node.args.last().is_some_and(is_none) {
            self.without_work += 1;
        }
        syn::visit::visit_expr_method_call(self, node);
    }

    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = node.func.as_ref() {
            let segments = path
                .path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect::<Vec<_>>();
            let without_work = node.args.last().is_some_and(is_none);
            match segments.as_slice() {
                [.., owner, name] if owner == "WorkGiven" && name == "for_command" => {
                    self.for_command += 1
                }
                [.., owner, name] if owner == "ProcessExecutionPolicy" && name == "new" => {
                    self.policies += 1;
                    self.without_work += usize::from(without_work);
                }
                [.., owner, name] if owner == "EdtSessionRequest" && name == "new" => {
                    self.without_work += usize::from(without_work);
                }
                [.., owner, _] if owner == "ExecutionContext" => self.contexts += 1,
                [.., name] if name == "stamp_dispatch" => self.stamps += 1,
                _ => {}
            }
        }
        syn::visit::visit_expr_call(self, node);
    }

    fn visit_expr_return(&mut self, node: &'ast syn::ExprReturn) {
        self.returns += 1;
        syn::visit::visit_expr_return(self, node);
    }

    fn visit_expr_try(&mut self, node: &'ast syn::ExprTry) {
        self.tries += 1;
        syn::visit::visit_expr_try(self, node);
    }
}

/// Упоминает ли макрос признак: тело макроса страж не разбирает, поэтому признаку там не
/// место. Строковые литералы не в счёт — `format!("{provider_dispatched}")` только печатает.
fn mentions_dispatch(tokens: &impl quote::ToTokens) -> bool {
    static STRINGS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#""(?:[^"\\]|\\.)*""#).expect("string literal pattern"));
    let text = tokens.to_token_stream().to_string();
    STRINGS
        .replace_all(&text, "")
        .split(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .any(|word| word == "provider_dispatched")
}

/// Макросы уровня элементов, чьи токены упоминают признак: `модуль::имя!`. Макросы внутри
/// тел смотрит `DispatchUse`.
struct DispatchMacros {
    module: Vec<String>,
    found: Vec<String>,
}

impl<'ast> syn::visit::Visit<'ast> for DispatchMacros {
    fn visit_item(&mut self, node: &'ast syn::Item) {
        if !item_has_cfg_test(node) {
            syn::visit::visit_item(self, node);
        }
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        if !has_cfg_test(&node.attrs) {
            syn::visit::visit_impl_item_fn(self, node);
        }
    }

    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        self.module.push(node.ident.to_string());
        syn::visit::visit_item_mod(self, node);
        self.module.pop();
    }

    fn visit_block(&mut self, _node: &'ast syn::Block) {}

    fn visit_item_macro(&mut self, node: &'ast syn::ItemMacro) {
        if mentions_dispatch(&node.mac.tokens) {
            let name = node
                .ident
                .as_ref()
                .map_or_else(|| normalize_tokens(&node.mac.path), ToString::to_string);
            self.found
                .push(format!("{}::{name}!", self.module.join("::")));
        }
    }

    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        if mentions_dispatch(&node.tokens) {
            self.found.push(format!(
                "{}::{}!",
                self.module.join("::"),
                normalize_tokens(&node.path)
            ));
        }
    }
}

fn dispatch_macros(index: &SourceIndex) -> Vec<String> {
    let mut finder = DispatchMacros {
        module: Vec::new(),
        found: Vec::new(),
    };
    for unit in &index.units {
        finder.module = unit.module.clone();
        for item in &unit.syntax.items {
            syn::visit::Visit::visit_item(&mut finder, item);
        }
    }
    finder.found.sort();
    finder.found
}

/// Формы, несущие признак: типы, перечисленные в единственном `carries_dispatch!`, по
/// последнему сегменту пути.
fn dispatch_forms(index: &SourceIndex) -> Vec<String> {
    let invocations = index
        .units
        .iter()
        .flat_map(|unit| &unit.syntax.items)
        .filter(|item| !item_has_cfg_test(item))
        .filter_map(|item| match item {
            syn::Item::Macro(item_macro) if item_macro.mac.path.is_ident("carries_dispatch") => {
                Some(&item_macro.mac)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        invocations.len(),
        1,
        "the forms that carry the flag are listed once, in carries_dispatch!"
    );
    let types = invocations[0]
        .parse_body_with(syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated)
        .expect("carries_dispatch! lists type paths");
    types
        .iter()
        .filter_map(|path| path.segments.last())
        .map(|segment| segment.ident.to_string())
        .collect()
}

/// Входы сценариев, чей ответ несёт признак: открытые функции сценариев и MCP, которые
/// возвращают такую форму и могут дать работу — берут контекст команды или заводят свою
/// отметку. Выводятся из типов, а не перечнем: вход, забывший штамп, не ускользнёт.
fn dispatch_entries(index: &SourceIndex, forms: &[String]) -> Vec<String> {
    let scenarios = path_of("crate::use_cases");
    let mcp = path_of("crate::mcp");
    let mut entries = index
        .functions
        .iter()
        .filter(|(_, function)| {
            function.module.starts_with(&scenarios) || function.module.starts_with(&mcp)
        })
        .filter(|(_, function)| matches!(function.item.vis, syn::Visibility::Public(_)))
        .filter(|(_, function)| {
            let output = normalize_tokens(&function.item.sig.output);
            output
                .split(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
                .any(|word| forms.iter().any(|form| form == word))
        })
        .filter(|(_, function)| {
            let inputs = normalize_tokens(&function.item.sig.inputs);
            inputs.contains("ExecutionContext")
                || DispatchUse::of(&function.item.block).for_command > 0
        })
        .map(|(path, _)| path.join("::"))
        .collect::<Vec<_>>();
    entries.sort();
    entries
}

/// Корень #312: сценарии решали признак сами, чаще всего постоянной `true` на всяком
/// отказе. Владелец теперь один — отметка работы команды: её ставит только платформа, а
/// значение признаку — только `stamp_dispatch` в хвосте входа сценария. Страж держит обе
/// стороны и узнаёт ту же ошибку под другим именем: решённое по месту значение в сценарии,
/// MCP, домене или CLI, составное присваивание, признак внутри макроса, `stamp_work` мимо
/// штампа, отметку вне платформы, свою отметку или свой контекст в сценарии, шаг, объявленный
/// «не работой», вне платформы, и вход, возвращающий форму с признаком без штампа в хвосте.
#[test]
fn provider_dispatched_takes_its_value_only_from_the_work_mark() {
    let index = SourceIndex::of_src();
    let scenarios = path_of("crate::use_cases");
    let mcp = path_of("crate::mcp");
    let cli = path_of("crate::cli");
    let domain = path_of("crate::domain");
    let platform = path_of("crate::platform");
    let result = path_of("crate::use_cases::result");
    let context = path_of("crate::use_cases::context");
    let owners_of_command_work = [context.clone(), path_of("crate::mcp::edt_syntax")];

    let mut violations = Vec::new();
    let mut stamping = Vec::new();
    for body in production_bodies(&index) {
        let site = format!("{}::{}", body.module.join("::"), body.context);
        let found = DispatchUse::of(body.block);
        let in_scenarios = body.module.starts_with(&scenarios) || body.module.starts_with(&mcp);
        let mut violation = |what: String| violations.push(format!("{site}: {what}"));
        if !found.decided.is_empty() {
            violation(format!("decides the flag: {:?}", found.decided));
        }
        if found.copies > 0 && in_scenarios {
            violation("copies another answer's flag".to_owned());
        }
        if found.copies > 0
            && !in_scenarios
            && !body.module.starts_with(&cli)
            && !body.module.starts_with(&domain)
        {
            violation("copies the flag outside the wire forms".to_owned());
        }
        if found.writes > 0 {
            violation("writes the flag through &mut or a compound assignment".to_owned());
        }
        if found.opaque_macros > 0 {
            violation("names the flag inside a macro the guard cannot read".to_owned());
        }
        if found.marks > 0 && !body.module.starts_with(&platform) {
            violation("marks the command's work outside the platform".to_owned());
        }
        if found.stamp_work > 0 && body.module != result {
            violation("stamps a form past stamp_dispatch".to_owned());
        }
        if found.for_command > 0 && !owners_of_command_work.contains(&body.module) {
            violation("creates its own work mark".to_owned());
        }
        if found.without_work > 0 && !body.module.starts_with(&platform) {
            violation("declares a step not to be the command's work".to_owned());
        }
        if found.policies > 0 && !body.module.starts_with(&platform) && body.module != context {
            violation("builds a process policy past the command's context".to_owned());
        }
        if found.contexts > 0 && body.module.starts_with(&scenarios) && body.module != context {
            violation("opens a second command context inside a scenario".to_owned());
        }
        if found.stamps > 0 {
            stamping.push(match &body.enclosing {
                Some(path) if body.owner.is_none() => path.join("::"),
                _ => site,
            });
        }
    }
    assert!(
        violations.is_empty(),
        "provider_dispatched must take its value only from the work mark:\n{}",
        violations.join("\n")
    );
    assert_eq!(
        dispatch_macros(&index),
        ["crate::use_cases::result::carries_dispatch!"],
        "only carries_dispatch! may name the flag in an item-level macro"
    );

    let entries = dispatch_entries(&index, &dispatch_forms(&index));
    stamping.sort();
    assert_eq!(
        stamping, entries,
        "the flag is stamped exactly at the entries whose answer carries it"
    );
    for entry in &entries {
        let function = &index.functions[&path_of(entry)].item;
        let found = DispatchUse::of(&function.block);
        let tail = match function.block.stmts.last() {
            Some(syn::Stmt::Expr(expr, None)) => Some(expr),
            _ => None,
        };
        assert!(
            (found.stamps, found.returns, found.tries) == (1, 0, 0)
                && tail.is_some_and(is_stamped_tail),
            "{entry} must leave through one exit, its tail, and stamp the work there"
        );
    }
}

/// Страж выше смотрит глазами этих поисков, поэтому они проверены на образце.
#[test]
fn the_dispatch_finder_sees_decisions_marks_and_exits() {
    let index = SourceIndex::from_sources(&[
        (
            "crate::use_cases::result",
            "macro_rules! carries_dispatch { ($t:ty) => { impl C for $t { fn stamp_work(&mut self, w: &W) { self.provider_dispatched = w.given(); } } }; }\n\
             carries_dispatch!(crate::domain::sample::R, crate::domain::sample::Q);",
        ),
        (
            "crate::use_cases::sample",
            "pub fn decides(ok: bool) -> R { R { provider_dispatched: ok } }\n\
             pub fn shorthand(provider_dispatched: bool) -> R { R { provider_dispatched } }\n\
             pub fn copies(r: R) -> R { R { provider_dispatched: r.provider_dispatched } }\n\
             pub fn assigns(mut r: R) -> R { r.provider_dispatched = true; r }\n\
             pub fn honest(mut r: R) -> Q { r.provider_dispatched = Some(false); Q { provider_dispatched: false, other: None } }\n\
             pub fn compounds(mut r: R, ok: bool) { r.provider_dispatched |= ok; }\n\
             pub fn borrows(mut r: R) { let flag = &mut r.provider_dispatched; *flag = true; }\n\
             pub fn marks(work: &WorkGiven) { work.mark_work_given(); }\n\
             pub fn stamps_by_hand(r: &mut R, w: &WorkGiven) { r.stamp_work(w); }\n\
             pub fn serves(p: P) -> P { let q = ProcessExecutionPolicy::new(a, b, c, None); P { work: None, ..p } }\n\
             pub fn queues() -> E { EdtSessionRequest::new(c, d, None) }\n\
             pub fn launches(r: &Runner) { r.spawn_managed(&q, Mode::Wait, None); }\n\
             pub fn opens() -> ExecutionContext { ExecutionContext::cli(C::X) }\n\
             pub fn logs(r: &R) { debug!(\"{}\", r.provider_dispatched); format!(\"{provider_dispatched}\"); }\n\
             pub fn hides(ok: bool) -> Vec<R> { vec![R { provider_dispatched: ok }] }\n\
             pub fn obscures() { weird!(provider_dispatched => true); }\n\
             pub fn exits(c: &ExecutionContext) -> UseCaseResult<R> { stamp_dispatch(run(c), c.work()) }\n\
             pub fn maps(c: &M) -> Result<UseCaseResult<Q>, T> { let w = WorkGiven::for_command(); run(c).map(|o| stamp_dispatch(o, &w)) }\n\
             pub fn branches(c: &ExecutionContext, d: bool) -> UseCaseResult<R> { if d { stamp_dispatch(run(c), c.work()) } else { run(c) } }\n\
             pub fn forgets(c: &ExecutionContext) -> UseCaseResult<R> { run(c) }\n\
             pub fn plans(p: &Plan) -> Result<X, UseCaseFailure<R>> { plan(p) }",
        ),
    ]);
    let found = |name: &str| {
        let function = &index.functions[&path_of(&format!("crate::use_cases::sample::{name}"))];
        DispatchUse::of(&function.item.block)
    };
    assert_eq!(found("decides").decided, ["ok"]);
    assert_eq!(found("shorthand").decided, ["provider_dispatched"]);
    assert_eq!(found("copies").copies, 1);
    assert_eq!(found("assigns").decided, ["true"]);
    let honest = found("honest");
    assert!(honest.decided.is_empty() && honest.copies == 0);
    assert_eq!(found("compounds").writes, 1);
    assert_eq!(found("borrows").writes, 1);
    assert_eq!(found("marks").marks, 1);
    assert_eq!(found("stamps_by_hand").stamp_work, 1);
    let serves = found("serves");
    assert_eq!((serves.without_work, serves.policies), (2, 1));
    assert_eq!(found("queues").without_work, 1);
    assert_eq!(found("launches").without_work, 1);
    assert_eq!(found("opens").contexts, 1);
    let logs = found("logs");
    assert!(
        logs.decided.is_empty() && logs.opaque_macros == 0,
        "a read is not a decision"
    );
    assert_eq!(found("hides").decided, ["ok"]);
    assert_eq!(found("obscures").opaque_macros, 1);
    assert_eq!(
        dispatch_macros(&index),
        ["crate::use_cases::result::carries_dispatch!"]
    );

    let forms = dispatch_forms(&index);
    assert_eq!(forms, ["R", "Q"]);
    assert_eq!(
        dispatch_entries(&index, &forms),
        [
            "crate::use_cases::sample::branches",
            "crate::use_cases::sample::exits",
            "crate::use_cases::sample::forgets",
            "crate::use_cases::sample::maps",
        ],
        "an entry is found by its answer and its context, the plan without one is not"
    );
    let tail = |name: &str| {
        let function = &index.functions[&path_of(&format!("crate::use_cases::sample::{name}"))];
        match function.item.block.stmts.last() {
            Some(syn::Stmt::Expr(expr, None)) => is_stamped_tail(expr),
            _ => false,
        }
    };
    assert!(tail("exits") && tail("maps"));
    assert!(!tail("branches") && !tail("forgets"));
}

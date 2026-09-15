mod guardrail_support;

use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};

use guardrail_support::{
    collect_rust_files, free_function_tokens, production_tokens, trait_impl_method_tokens,
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
        "fn execution_policy",
    );
    let source_tools = Regex::new(r#""([a-z_]+)""#)
        .expect("regex")
        .captures_iter(source_section)
        .map(|capture| capture[1].to_owned())
        .collect::<Vec<_>>();

    let contract = read("spec/arch/contracts/CTR.MCP.PUBLISHED-TOOL-SURFACE.md");
    let contract_section = extract_between(
        &contract,
        "Опубликованы восемь инструментов:",
        "Состав меняется только вместе с версией этой формы.",
    );
    let contract_tools = extract_backticked_items(contract_section);

    let expected = EXPECTED_MCP_TOOLS
        .iter()
        .map(|tool| (*tool).to_owned())
        .collect::<Vec<_>>();

    assert_eq!(source_tools, expected);
    assert_eq!(contract_tools, expected);
}

#[test]
fn public_command_adapters_keep_workspace_lock_boundary() {
    for function in [
        "execute_extensions",
        "execute_init",
        "execute_build",
        "execute_test",
        "execute_load",
        "execute_dump",
        "execute_infobase_configuration_export",
        "execute_infobase_dump",
        "execute_convert",
        "execute_artifacts",
        "execute_syntax",
        "execute_launch",
    ] {
        let window = free_function_tokens(repo_path("src/cli/execute.rs").as_path(), function);
        assert!(
            window.contains("with_cli_workspace_lock(")
                || window.contains("with_cli_workspace_lock_observed("),
            "{function} must keep the CLI workspace-lock boundary"
        );
    }

    for function in [
        "build_project",
        "run_tests",
        "dump_config",
        "launch_app",
        "check_syntax",
    ] {
        let window = trait_impl_method_tokens(
            repo_path("src/mcp/port.rs").as_path(),
            "McpUseCasePort",
            "DefaultMcpUseCasePort",
            function,
        );
        assert!(
            window.contains("with_workspace_lock("),
            "{function} must keep the MCP workspace-lock boundary"
        );
    }
}

#[test]
fn change_checklist_covers_mcp_workspace_lock_and_config_contract() {
    let checklist = read("spec/architecture/change-checklist.md");
    for required in [
        "## Изменение MCP public surface",
        "## Новая public CLI/MCP команда, работающая с `workPath`",
        "## Новый public config field, `source-set` type или `infobase` subtree",
        "src/config/model.rs",
        "src/config/validate.rs",
        "spec/arch/README.md",
    ] {
        assert!(
            checklist.contains(required),
            "checklist must mention '{required}'"
        );
    }
}

/// Вложенные шаги не берут блокировку повторно: замок рабочего каталога живёт на
/// границе адаптера, а сценарии зовут друг друга через входы без замка. Второй захват
/// изнутри дал бы «занято» самому себе.
#[test]
fn nested_orchestration_never_acquires_the_workspace_lock_inside_use_cases() {
    let root = repo_path("src/use_cases");
    for file in collect_rust_files(&root) {
        // `workspace_lock.rs` реализует замок, `transport.rs` — граница адаптера, где он
        // берётся один раз за команду. Всё остальное в слое сценариев работает под ним.
        if matches!(
            file.file_name().and_then(|name| name.to_str()),
            Some("workspace_lock.rs") | Some("transport.rs")
        ) {
            continue;
        }
        let production = production_tokens(&file);
        assert!(
            !production.contains("acquire_workspace_lock("),
            "{} takes the workspace lock inside a use case; nested steps run under the caller's lock",
            file.display()
        );
    }

    // `test` строит перед прогоном тем же сценарием сборки, не выходя на границу адаптера.
    let run_tests = read("src/use_cases/run_tests/coordinator.rs");
    assert!(
        run_tests.contains("build_project::execute("),
        "run_tests must reuse the build use case directly, under the lock already held by the caller"
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

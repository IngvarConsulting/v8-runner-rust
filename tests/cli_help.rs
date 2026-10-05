mod support;

/// Перечень прежних имён — тот же файл, что держит разбор: в тесте он не повторяется.
#[path = "../src/cli/synonyms.rs"]
mod synonyms;

use std::collections::{BTreeMap, BTreeSet};

use support::v8_runner_command;
use synonyms::{Previous, SYNONYMS};

#[test]
fn root_help_splits_commands_and_global_options() {
    let output = v8_runner_command()
        .args(["--help"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Commands:"));
    assert!(stdout.contains("Global options:"));
    assert!(stdout.contains("Print application version"));
    assert!(stdout.contains("Send configured source-sets to the infobase"));
    assert!(stdout.contains("--json-message"));
}

fn help(path: &[String], flag: &str) -> String {
    let output = v8_runner_command()
        .args(path)
        .arg(flag)
        .output()
        .expect("run help");
    assert!(
        output.status.success(),
        "{path:?} {flag}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf-8 help")
}

/// Подкоманды из раздела `Commands:` вместе с видимыми псевдонимами.
fn listed_subcommands(help: &str) -> BTreeMap<String, Vec<String>> {
    help.lines()
        .skip_while(|line| *line != "Commands:")
        .skip(1)
        .take_while(|line| !line.trim().is_empty())
        .filter(|line| line.starts_with("  ") && !line.starts_with("   "))
        .map(|line| {
            let name = line.split_whitespace().next().expect("name").to_owned();
            let aliases = line
                .split_once("[aliases: ")
                .or_else(|| line.split_once("[alias: "))
                .and_then(|(_, rest)| rest.split_once(']'))
                .map(|(aliases, _)| aliases.split(", ").map(str::to_owned).collect())
                .unwrap_or_default();
            (name, aliases)
        })
        .collect()
}

/// Длинные ключи, которые справка перечисляет как ключи этой команды.
fn listed_keys(help: &str) -> BTreeSet<String> {
    help.lines()
        .map(str::trim_start)
        .filter(|line| line.starts_with('-'))
        .flat_map(|line| {
            line.split_whitespace()
                .take_while(|token| token.starts_with('-'))
                .filter_map(|token| token.strip_prefix("--"))
                .map(|name| name.trim_end_matches(',').to_owned())
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Упоминает ли текст ключ `--name` целым словом.
fn mentions_key(help: &str, name: &str) -> bool {
    let needle = format!("--{name}");
    let word = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    help.match_indices(&needle).any(|(start, _)| {
        let before = help[..start].chars().next_back();
        let after = help[start + needle.len()..].chars().next();
        !before.is_some_and(word) && !after.is_some_and(word)
    })
}

/// Значения, которые справка перечисляет у ключа `--key`.
fn listed_values(help: &str, key: &str) -> Option<Vec<String>> {
    let lines = help.lines().collect::<Vec<_>>();
    let head = format!("--{key} <");
    let start = lines.iter().position(|line| line.contains(&head))?;
    let block = lines[start..]
        .iter()
        .enumerate()
        .take_while(|(index, line)| {
            *index == 0 || !(line.trim_start().starts_with('-') || line.trim().is_empty())
        })
        .map(|(_, line)| *line)
        .collect::<Vec<_>>()
        .join("\n");
    let values = block
        .split_once("[possible values: ")
        .and_then(|(_, rest)| rest.split_once(']'))
        .map(|(values, _)| values.split(", ").map(str::to_owned).collect::<Vec<_>>())
        .unwrap_or_default();
    Some(values)
}

/// Справка всех уровней — корня и каждой видимой подкоманды, краткая и полная — печатает
/// только словарь сайта: ни одно прежнее имя команды, ключа или значения из перечня
/// `src/cli/synonyms.rs` в ней не появляется (`INV.CLI.A-HIDDEN-SYNONYM-IS-ABSENT-FROM-HELP`).
#[test]
fn no_help_at_any_level_prints_a_previous_name() {
    let mut pending = vec![Vec::<String>::new()];
    let mut visited = BTreeSet::new();
    while let Some(path) = pending.pop() {
        for flag in ["-h", "--help"] {
            let text = help(&path, flag);
            let keys = listed_keys(&text);
            let subcommands = listed_subcommands(&text);
            for synonym in SYNONYMS {
                let here = synonym.command == path.as_slice();
                let shown = match synonym.previous {
                    Previous::Command(previous) => {
                        here && subcommands.iter().any(|(name, aliases)| {
                            name == previous || aliases.iter().any(|alias| alias == previous)
                        })
                    }
                    // Ключ, который у этой команды есть под новым смыслом (`upload --mode`),
                    // справка печатает по праву; иначе прежнее имя не появляется даже в прозе.
                    Previous::Key(previous) => {
                        (here && keys.contains(previous))
                            || (!keys.contains(previous) && mentions_key(&text, previous))
                    }
                    Previous::Value { key, value } if here => listed_values(&text, key)
                        .unwrap_or_else(|| panic!("{path:?} {flag} has no --{key}:\n{text}"))
                        .iter()
                        .any(|listed| listed == value),
                    Previous::Value { .. } => false,
                };
                assert!(
                    !shown,
                    "`v8-runner {} {flag}` prints the previous name {:?} instead of {}:\n{text}",
                    path.join(" "),
                    synonym.previous,
                    synonym.current
                );
            }
            if flag == "--help" {
                for name in subcommands.into_keys().filter(|name| name != "help") {
                    let mut inner = path.clone();
                    inner.push(name);
                    pending.push(inner);
                }
            }
        }
        visited.insert(path);
    }

    // Каждая строка перечня проверена справкой той команды, где живёт.
    for synonym in SYNONYMS {
        let path = synonym
            .command
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>();
        assert!(
            visited.contains(&path),
            "{:?} lives under {path:?}, which no help lists",
            synonym.previous
        );
    }
    assert!(visited.len() > SYNONYMS.len(), "{visited:?}");
}

#[test]
fn root_version_flag_prints_application_version() {
    let output = v8_runner_command()
        .args(["--version"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.trim(),
        format!("v8-runner {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn config_init_help_separates_global_and_command_options() {
    let output = v8_runner_command()
        .args(["config", "init", "--help"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Command options:"));
    assert!(stdout.contains("Global options:"));
    assert!(stdout.contains("--output <OUTPUT>"));
    assert!(!stdout.contains("--file <FILE>"));
    assert!(stdout.contains("--json-message"));
}

#[test]
fn push_help_exposes_source_set_selector() {
    let output = v8_runner_command()
        .args(["push", "--help"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Command options:"));
    // Набор называет позиционный аргумент; прежний ключ принимается, но в справке его нет.
    assert!(
        stdout.contains("Usage: v8-runner push [OPTIONS] [SET]"),
        "{stdout}"
    );
    assert!(!stdout.contains("--source-set"), "{stdout}");
    assert!(stdout.contains("--full"));
    assert!(stdout.contains("--json-message"));
    // Прежнее имя ключа принимается, но в справке его нет.
    assert!(!stdout.contains("--full-rebuild"), "{stdout}");
}

#[test]
fn test_help_exposes_no_build_option() {
    let output = v8_runner_command()
        .args(["test", "--help"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--no-push"));
}

#[test]
fn dump_help_clarifies_object_selector_compatibility() {
    let output = v8_runner_command()
        .args(["dump", "--help"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("canonical TYPE:NAME selectors"));
    assert!(stdout.contains("legacy TYPE.NAME selectors are accepted for compatibility"));
}

/// В проекте EDT любая выгрузка заменяет каталог проекта: справка не обещает «поверх» и
/// называет оба выхода из отказа сторожа.
#[test]
fn pull_help_says_every_edt_dump_replaces_the_project() {
    for flag in ["-h", "--help"] {
        let help = help(&["pull".to_owned()], flag);
        for phrase in [
            "EDT-format project: every dump replaces the whole project directory",
            "uncommitted work there makes the dump refuse",
            "commit or stash it and repeat, or repeat the same command with `--force` added",
            "in a Designer-format project nothing else in it is touched",
        ] {
            assert!(help.contains(phrase), "{flag} must say {phrase:?}:\n{help}");
        }
        // Голый `pull --force` в выходе из отказа теряет набор и бьёт в другой каталог.
        assert!(
            !help.contains("run `pull --force`"),
            "{flag} must not offer a bare `pull --force`:\n{help}"
        );
    }
}

#[test]
fn tools_download_help_exposes_tool_commands() {
    let output = v8_runner_command()
        .args(["tools", "download", "--help"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Global options:"));
    assert!(stdout.contains("Commands:"));
    assert!(stdout.contains("yaxunit"));
    assert!(stdout.contains("vanessa"));
    assert!(stdout.contains("client-mcp"));
    assert!(!stdout.contains("--extensions"));
}

#[test]
fn tools_download_extension_help_exposes_sources_flag() {
    let output = v8_runner_command()
        .args(["tools", "download", "yaxunit", "--help"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Command options:"));
    assert!(stdout.contains("--sources"));
    assert!(stdout.contains("--force"));
}

#[test]
fn launch_help_uses_output_path_name_and_global_json_selector() {
    let output = v8_runner_command()
        .args(["launch", "--help"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Command options:"));
    assert!(stdout.contains("Global options:"));
    assert!(stdout.contains("--output <OUTPUT>"));
    assert!(stdout.contains("--stderr-output <STDERR_OUTPUT>"));
    assert!(stdout.contains("--wait-for-exit"));
    assert!(stdout.contains("--wait-timeout-ms <WAIT_TIMEOUT_MS>"));
    assert!(!stdout.contains("--out <OUT>"));
    assert!(!stdout.contains("--mode <MODE>"));
    assert!(stdout.contains("--json-message"));
}

#[test]
fn test_help_does_not_expose_direct_launch_wait_options() {
    let output = v8_runner_command()
        .args(["test", "--help"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("--c <C>"));
    assert!(!stdout.contains("--execute <EXECUTE>"));
    assert!(!stdout.contains("--output <OUTPUT>"));
    assert!(!stdout.contains("--stderr-output"));
    assert!(!stdout.contains("--wait-for-exit"));
    assert!(!stdout.contains("--wait-timeout-ms"));
}

#[test]
fn make_help_keeps_output_path_under_command_options() {
    let output = v8_runner_command()
        .args(["make", "--help"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Command options:"));
    assert!(stdout.contains("Global options:"));
    assert!(stdout.contains("--output <OUTPUT>"));
    assert!(stdout.contains("--json-message"));
}

#[test]
fn convert_help_uses_output_target_root_name() {
    let output = v8_runner_command()
        .args(["convert", "--help"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Command options:"));
    assert!(stdout.contains("Global options:"));
    assert!(stdout.contains("--output <OUTPUT>"));
    assert!(
        stdout.contains("Usage: v8-runner convert [OPTIONS] [SET]"),
        "{stdout}"
    );
    assert!(!stdout.contains("--source-set"), "{stdout}");
    assert!(stdout.contains("--json-message"));
}

#[test]
fn infobase_configuration_export_help_fixes_the_exact_grammar() {
    let output = v8_runner_command()
        .args(["infobase", "configuration", "export", "--help"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[SET]"), "{stdout}");
    assert!(stdout.contains("--state <STATE>"));
    // Словарь называет состояние базы данных `db`; прежние значения в справке не печатаются.
    assert!(stdout.contains("[possible values: db]"), "{stdout}");
    assert!(stdout.contains("--extension <EXTENSION>"));
    assert!(stdout.contains("--output <OUTPUT>"));
    assert!(stdout.contains("--dry-run"));
    assert!(!stdout.contains("--provider"));
    assert!(!stdout.contains("--engine"));
}

#[test]
fn infobase_dump_help_calls_dt_a_transfer_file_not_a_backup() {
    let output = v8_runner_command()
        .args(["infobase", "dump", "--help"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--dry-run"));
    assert!(stdout.contains("--output <OUTPUT>"));
    assert!(stdout.contains("not a backup"));
}

/// Файл пакета у `upload` обязателен и назван позиционно: прежний ключ `--path`
/// принимается, но ни в строке вызова, ни в списке ключей его нет.
#[test]
fn upload_help_shows_the_package_file_as_required_positional() {
    let output = v8_runner_command()
        .args(["upload", "--help"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Usage: v8-runner upload [OPTIONS] <FILE>"),
        "{stdout}"
    );
    assert!(!stdout.contains("--path"), "{stdout}");
}

/// Набор у `pull` и `make` называет позиционный аргумент: прежний ключ `--source-set`
/// в справке не печатается.
#[test]
fn pull_and_make_help_name_the_source_set_positionally() {
    for command in ["pull", "make"] {
        let output = v8_runner_command()
            .args([command, "--help"])
            .output()
            .expect("run command");

        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains(&format!("Usage: v8-runner {command} [OPTIONS]")),
            "{stdout}"
        );
        assert!(stdout.contains("[SET]"), "{stdout}");
        assert!(!stdout.contains("--source-set"), "{stdout}");
    }
}

//! `reset`: отбросить непринятое (#235).
//!
//! Поддельная платформа держит состояние базы файлами каталога `state`: `main` — основная
//! конфигурация, `db` — конфигурация базы данных, `token` — поколение у Конфигуратора,
//! `ibcmd-token` — у `ibcmd`; у расширения `ext` — те же имена с хвостом `-ext`. Загрузка
//! меняет `main`, применение копирует `main` в `db`, откат — `db` в `main`; сохранения
//! `/DumpCfg` и `/DumpDBCfg` (`config save [--db]`) отдают `main` и `db`, и признак
//! непринятого — их неравенство, как у платформы. Откат меняет поколение на `rollback-token`
//! (`ibcmd-rollback-token`), если он лежит, и отказывает при `rollback-fails`; сохранение
//! конфигурации базы данных отказывает при `unknown`. Состав расширений — файл `installed`.
#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::Value;
use support::command_data::assert_data_matches_its_command_form;
use support::{temp_workspace, v8_runner_command, write_shell_script};

const FIRST: &str = "1111111111111111111111111111111111111111";
const SECOND: &str = "2222222222222222222222222222222222222222";
const THIRD: &str = "3333333333333333333333333333333333333333";

fn designer(root: &Path) -> String {
    format!(
        r#"printf 'designer %s\n' "$*" >> '{calls}'
state='{state}'
out=''
target=''
previous=''
for arg in "$@"; do
  if [ "$previous" = '/Out' ]; then out="$arg"; fi
  if [ "$previous" = '/DumpCfg' ] || [ "$previous" = '/DumpDBCfg' ]; then target="$arg"; fi
  previous="$arg"
done
suffix=''
case "$*" in *'-Extension ext'*) suffix='-ext' ;; esac
case "$*" in
  *'/GetConfigGenerationID'*)
    if [ -f "$state/token$suffix" ]; then cat "$state/token$suffix" > "$out"; fi
    exit 0 ;;
  *'/DumpDBCfgList'*)
    cat "$state/installed" > "$out"
    exit 0 ;;
  *'/DumpDBCfg'*)
    if [ -f "$state/unknown" ]; then exit 1; fi
    cat "$state/db$suffix" > "$target"
    exit 0 ;;
  *'/DumpCfg'*)
    cat "$state/main$suffix" > "$target"
    exit 0 ;;
  *'/LoadConfigFromFiles'*)
    printf 'x' >> "$state/main$suffix" ;;
  *'/UpdateDBCfg'*)
    cp "$state/main$suffix" "$state/db$suffix" ;;
  *'/RollbackCfg'*)
    if [ -f "$state/rollback-fails" ]; then
      printf 'Ошибка блокировки информационной базы для конфигурирования.' > "$out"
      exit 1
    fi
    cp "$state/db$suffix" "$state/main$suffix"
    if [ -f "$state/rollback-token" ]; then mv "$state/rollback-token" "$state/token$suffix"; fi
    if [ -f "$state/ibcmd-rollback-token" ]; then mv "$state/ibcmd-rollback-token" "$state/ibcmd-token$suffix"; fi ;;
esac
if [ -n "$out" ]; then : > "$out"; fi
exit 0"#,
        calls = root.join("calls.log").display(),
        state = root.join("state").display(),
    )
}

fn ibcmd(root: &Path) -> String {
    format!(
        r#"printf 'ibcmd %s\n' "$*" >> '{calls}'
state='{state}'
last=''
for arg in "$@"; do last="$arg"; done
suffix=''
case "$*" in *'--extension ext'*) suffix='-ext' ;; esac
case "$*" in
  *'config generation-id'*)
    if [ -f "$state/ibcmd-token$suffix" ]; then cat "$state/ibcmd-token$suffix"; fi
    exit 0 ;;
  *'config save'*)
    case "$*" in
      *' --db '*) cat "$state/db$suffix" > "$last" ;;
      *) cat "$state/main$suffix" > "$last" ;;
    esac
    exit 0 ;;
  *'config import'*)
    printf 'x' >> "$state/main$suffix"
    exit 0 ;;
  *'config apply'*)
    cp "$state/main$suffix" "$state/db$suffix"
    exit 0 ;;
  *'config reset'*)
    if [ -f "$state/rollback-fails" ]; then exit 255; fi
    cp "$state/db$suffix" "$state/main$suffix"
    if [ -f "$state/ibcmd-rollback-token" ]; then mv "$state/ibcmd-rollback-token" "$state/ibcmd-token$suffix"; fi
    exit 0 ;;
esac
exit 0"#,
        calls = root.join("calls.log").display(),
        state = root.join("state").display(),
    )
}

struct Project {
    dir: tempfile::TempDir,
    config: PathBuf,
    bin: PathBuf,
}

impl Project {
    /// Проект с набором `main` и файловой базой `origin`; в базе всё применено.
    fn new() -> Self {
        Self::with_sets(&[("main", "CONFIGURATION", "sources")], "")
    }

    /// Проект с наборами `sets` (имя, тип, каталог) и ключами `providers` поверх тех, что
    /// ставят Конфигуратор первым.
    fn with_sets(sets: &[(&str, &str, &str)], providers: &str) -> Self {
        let dir = temp_workspace();
        let root = dir.path();
        let mut declared = String::new();
        for (name, kind, path) in sets {
            let sources = root.join(path);
            if kind.starts_with("EXTERNAL") {
                fs::create_dir_all(sources.join("Alpha")).expect("external");
                fs::write(
                    sources.join("Alpha.xml"),
                    "<ExternalDataProcessor><Properties><Name>Alpha</Name></Properties></ExternalDataProcessor>",
                )
                .expect("descriptor");
                fs::write(sources.join("Alpha").join("Module.bsl"), "// module").expect("module");
            } else {
                fs::create_dir_all(&sources).expect("sources");
                fs::write(sources.join("Configuration.xml"), "<Configuration/>\n").expect("source");
                fs::write(sources.join("Module.bsl"), "Procedure A()\nEndProcedure\n")
                    .expect("module");
            }
            declared.push_str(&format!(
                "  - name: {name}\n    type: {kind}\n    path: {path}\n"
            ));
        }
        let bin = root.join("bin");
        fs::create_dir_all(&bin).expect("bin");
        write_shell_script(&bin.join("1cv8"), &designer(root));
        write_shell_script(&bin.join("ibcmd"), &ibcmd(root));
        let state = root.join("state");
        fs::create_dir_all(&state).expect("state");
        for (file, text) in [
            ("main", "applied"),
            ("db", "applied"),
            ("main-ext", "applied"),
            ("db-ext", "applied"),
            ("installed", ""),
        ] {
            fs::write(state.join(file), text).expect("state");
        }
        let project = Self {
            config: root.join("v8project.yaml"),
            bin,
            dir,
        };
        project.write_config(&declared, providers);
        fs::write(
            project.root().join("v8project.local.yaml"),
            "infobases:\n  origin:\n    connection: 'File=ib'\n",
        )
        .expect("local layer");
        project.generation(FIRST);
        project.ibcmd_generation(FIRST);
        project
    }

    fn write_config(&self, declared: &str, providers: &str) {
        let leads = support::DESIGNER_LEADS.replace("providers:\n", "");
        fs::write(
            &self.config,
            format!(
                "workPath: work\nformat: DESIGNER\nproviders:\n{leads}{providers}source-set:\n{declared}tools:\n  platform:\n    path: '{}'\n",
                self.bin.join("1cv8").display(),
            ),
        )
        .expect("config");
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn state(&self, name: &str) -> PathBuf {
        self.root().join("state").join(name)
    }

    fn generation(&self, token: &str) {
        fs::write(self.state("token"), format!("{token}\r\n")).expect("token");
        fs::write(self.state("token-ext"), format!("{token}\r\n")).expect("token");
    }

    fn ibcmd_generation(&self, token: &str) {
        fs::write(self.state("ibcmd-token"), format!("{token}\n")).expect("token");
    }

    fn mark(&self, name: &str) {
        fs::write(self.state(name), "").expect("mark");
    }

    fn installed(&self, names: &str) {
        fs::write(self.state("installed"), names).expect("installed");
    }

    fn run(&self, args: &[&str]) -> Output {
        v8_runner_command()
            .arg("--config")
            .arg(&self.config)
            .arg("--json-message")
            .args(args)
            .output()
            .expect("run CLI")
    }

    fn calls(&self) -> String {
        fs::read_to_string(self.root().join("calls.log")).unwrap_or_default()
    }

    fn forget_calls(&self) {
        let _ = fs::remove_file(self.root().join("calls.log"));
    }

    fn edit(&self) {
        let module = self.root().join("sources").join("Module.bsl");
        let text = fs::read_to_string(&module).expect("module");
        fs::write(&module, format!("{text}// edited\n")).expect("edit");
    }

    fn memory(&self) -> PathBuf {
        self.root().join("work").join("infobases").join("origin")
    }

    fn ledger(&self) -> Value {
        fs::read_to_string(self.memory().join("generation.json"))
            .map(|text| serde_json::from_str::<Value>(&text).expect("ledger json"))
            .unwrap_or(Value::Null)
    }

    fn record(&self) -> Value {
        self.ledger()["main"].clone()
    }

    fn hashes(&self) -> Option<Vec<u8>> {
        fs::read(self.memory().join("hashes").join("main.redb")).ok()
    }
}

fn envelope(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "no json envelope: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn succeeded(output: &Output) -> Value {
    let payload = envelope(output);
    assert!(output.status.success(), "{payload}");
    payload
}

fn refused(output: &Output) -> Value {
    let payload = envelope(output);
    assert!(!output.status.success(), "{payload}");
    payload
}

/// Строки вызовов отката.
fn rollbacks(calls: &str) -> Vec<&str> {
    calls
        .lines()
        .filter(|line| line.contains("/RollbackCfg") || line.contains("config reset"))
        .collect()
}

/// Главный ход: `push --no-apply`, затем `reset` — непринятое отброшено, память исходников
/// пуста, запись поколения переписана ответом после отката и снят признак «не применено»;
/// `status --deep` непринятого не видит, а следующая отправка грузит отброшенное заново.
#[test]
fn reset_discards_a_push_without_apply_and_the_next_push_loads_it_again() {
    let project = Project::new();
    succeeded(&project.run(&["push", "--force"]));
    project.edit();
    succeeded(&project.run(&["push", "--no-apply"]));
    assert_eq!(project.record()["applied"], false);
    fs::write(project.state("rollback-token"), format!("{SECOND}\r\n")).expect("token");
    project.forget_calls();

    let reset = succeeded(&project.run(&["reset"]));

    assert_eq!(reset["command"], "reset", "{reset}");
    assert_data_matches_its_command_form(&reset, "reset");
    let data = &reset["data"];
    assert_eq!(data["source_set"], "main", "{reset}");
    assert_eq!(data["purpose"], "CONFIGURATION", "{reset}");
    assert_eq!(data["outcome"], "discarded", "{reset}");
    assert_eq!(data["hash_memory"], "replaced", "{reset}");
    assert_eq!(data["generation"], "recorded", "{reset}");
    assert_eq!(data["provider_dispatched"], true, "{reset}");
    assert_eq!(data["provider"]["selected"], "designer", "{reset}");
    let calls = project.calls();
    let order: Vec<usize> = [
        "/DumpCfg",
        "/DumpDBCfg",
        "/GetConfigGenerationID",
        "/RollbackCfg",
    ]
    .iter()
    .map(|call| {
        calls
            .find(call)
            .unwrap_or_else(|| panic!("{call}: {calls}"))
    })
    .chain(calls.rfind("/GetConfigGenerationID"))
    .collect();
    assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{calls}");
    assert!(!calls.contains("-Extension"), "{calls}");
    assert!(!calls.contains("/UpdateDBCfg"), "{calls}");
    assert_eq!(project.record()["token"], SECOND, "{}", project.record());
    assert!(
        project.record().get("applied").is_none(),
        "{}",
        project.record()
    );
    assert_eq!(project.record()["tool"], "designer");

    let status = succeeded(&project.run(&["status", "--deep"]));
    let main = &status["data"]["infobases"][0]["source_sets"][0];
    assert_eq!(main["base"]["unapplied"], false, "{status}");
    assert_eq!(main["base"]["comparison"], "unchanged", "{status}");

    project.forget_calls();
    let pushed = succeeded(&project.run(&["push"]));
    let step = &pushed["data"]["steps"][0];
    assert_eq!(step["mode"], "full", "{pushed}");
    assert_eq!(step["applied"], true, "{pushed}");
    assert!(
        project.calls().contains("/LoadConfigFromFiles"),
        "{}",
        project.calls()
    );
}

/// Непринятого нет — отката нет: ответ это называет, а память и запись не тронуты.
#[test]
fn reset_with_nothing_unapplied_rolls_nothing_back() {
    let project = Project::new();
    succeeded(&project.run(&["push", "--force"]));
    let hashes = project.hashes();
    let ledger = project.ledger();
    project.forget_calls();

    let reset = succeeded(&project.run(&["reset"]));

    assert_eq!(reset["data"]["outcome"], "nothing_to_discard", "{reset}");
    assert!(reset["data"].get("hash_memory").is_none(), "{reset}");
    assert!(reset["data"].get("generation").is_none(), "{reset}");
    assert!(
        rollbacks(&project.calls()).is_empty(),
        "{}",
        project.calls()
    );
    assert_eq!(project.hashes(), hashes);
    assert_eq!(project.ledger(), ledger);
}

/// Признак непринятого не получен — отказ до отката; память и запись не тронуты.
#[test]
fn reset_without_the_unapplied_state_is_refused_before_the_rollback() {
    let project = Project::new();
    succeeded(&project.run(&["push", "--force"]));
    project.edit();
    succeeded(&project.run(&["push", "--no-apply"]));
    let hashes = project.hashes();
    let ledger = project.ledger();
    project.mark("unknown");
    project.forget_calls();

    let reset = refused(&project.run(&["reset"]));

    assert_eq!(reset["data"]["outcome"], "failed", "{reset}");
    assert_eq!(reset["data"]["provider_dispatched"], true, "{reset}");
    assert!(reset["data"].get("hash_memory").is_none(), "{reset}");
    assert!(
        reset["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains(
                "the unapplied state of the main configuration of source-set 'main' is not known"
            ),
        "{reset}"
    );
    assert!(
        rollbacks(&project.calls()).is_empty(),
        "{}",
        project.calls()
    );
    assert_eq!(project.hashes(), hashes);
    assert_eq!(project.ledger(), ledger);
}

/// Без набора откатывается только основная конфигурация, даже когда непринятое есть и у
/// расширения; расширение откатывается своим набором.
#[test]
fn reset_without_a_set_rolls_back_only_the_main_configuration() {
    let project = Project::with_sets(
        &[
            ("main", "CONFIGURATION", "sources"),
            ("ext", "EXTENSION", "ext"),
        ],
        "",
    );
    project.installed("ext\n");
    succeeded(&project.run(&["push", "--force"]));
    fs::write(project.state("main"), "loaded").expect("state");
    fs::write(project.state("main-ext"), "loaded").expect("state");
    project.forget_calls();

    let main = succeeded(&project.run(&["reset"]));

    assert_eq!(main["data"]["source_set"], "main", "{main}");
    assert_eq!(main["data"]["outcome"], "discarded", "{main}");
    let calls = project.calls();
    assert_eq!(rollbacks(&calls).len(), 1, "{calls}");
    assert!(!calls.contains("-Extension"), "{calls}");
    assert_eq!(
        fs::read_to_string(project.state("main-ext")).expect("ext"),
        "loaded"
    );

    project.forget_calls();
    let extension = succeeded(&project.run(&["reset", "ext"]));

    assert_eq!(extension["data"]["source_set"], "ext", "{extension}");
    assert_eq!(extension["data"]["purpose"], "EXTENSION", "{extension}");
    assert_eq!(extension["data"]["outcome"], "discarded", "{extension}");
    let calls = project.calls();
    assert!(calls.contains("/DumpDBCfgList"), "{calls}");
    let rolled = rollbacks(&calls);
    assert_eq!(rolled.len(), 1, "{calls}");
    assert!(rolled[0].contains("/RollbackCfg -Extension ext"), "{calls}");
    assert_eq!(
        fs::read_to_string(project.state("main-ext")).expect("ext"),
        fs::read_to_string(project.state("db-ext")).expect("ext")
    );
}

/// Набор расширения, которого нет в базе: отказ до признака и отката.
#[test]
fn reset_of_an_extension_that_is_not_installed_is_refused_before_work() {
    let project = Project::with_sets(
        &[
            ("main", "CONFIGURATION", "sources"),
            ("ext", "EXTENSION", "ext"),
        ],
        "",
    );

    let reset = refused(&project.run(&["reset", "ext"]));

    assert_eq!(reset["error"]["kind"], "validation", "{reset}");
    assert!(
        reset["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("is not installed in the infobase"),
        "{reset}"
    );
    let calls = project.calls();
    assert!(!calls.contains("/DumpCfg"), "{calls}");
    assert!(rollbacks(&calls).is_empty(), "{calls}");
}

/// Внешний набор в базу не идёт: отказ до платформы.
#[test]
fn reset_of_an_external_set_is_refused() {
    let project = Project::with_sets(
        &[
            ("main", "CONFIGURATION", "sources"),
            ("epf", "EXTERNAL_DATA_PROCESSORS", "epf"),
        ],
        "",
    );

    let reset = refused(&project.run(&["reset", "epf"]));

    assert_eq!(reset["error"]["kind"], "validation", "{reset}");
    assert!(
        reset["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("holds external files"),
        "{reset}"
    );
    assert!(project.calls().is_empty(), "{}", project.calls());
}

/// В проекте без набора основной конфигурации — у него одни внешние наборы: набор
/// расширения без основной конфигурации проект не объявит — `reset` без набора отказывает до
/// платформы.
#[test]
fn reset_without_a_configuration_set_is_refused() {
    let project = Project::with_sets(&[("epf", "EXTERNAL_DATA_PROCESSORS", "epf")], "");

    let reset = refused(&project.run(&["reset"]));

    assert_eq!(reset["error"]["kind"], "validation", "{reset}");
    assert!(
        reset["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("no CONFIGURATION source-set"),
        "{reset}"
    );
    assert!(project.calls().is_empty(), "{}", project.calls());
}

/// Откат не удался: отказ платформы с подсказкой об открытом Конфигураторе; память
/// исходников уже пуста — она заменяется до отката, — и следующая отправка грузит всё.
#[test]
fn a_failed_rollback_answers_a_platform_failure_and_the_next_push_loads_everything() {
    let project = Project::new();
    succeeded(&project.run(&["push", "--force"]));
    project.edit();
    succeeded(&project.run(&["push", "--no-apply"]));
    project.mark("rollback-fails");
    project.forget_calls();

    let reset = refused(&project.run(&["reset"]));

    assert_eq!(reset["error"]["kind"], "platform", "{reset}");
    assert_eq!(reset["data"]["outcome"], "failed", "{reset}");
    assert_eq!(reset["data"]["hash_memory"], "replaced", "{reset}");
    let message = reset["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("close it and run reset again"),
        "{message}"
    );
    assert_eq!(rollbacks(&project.calls()).len(), 1, "{}", project.calls());

    fs::remove_file(project.state("rollback-fails")).expect("unmark");
    project.forget_calls();
    let pushed = succeeded(&project.run(&["push"]));
    assert_ne!(pushed["data"]["steps"][0]["mode"], "skipped", "{pushed}");
    assert!(
        project.calls().contains("/LoadConfigFromFiles"),
        "{}",
        project.calls()
    );
}

/// Запись сделал `ibcmd`, откатывает Конфигуратор: поколение после отката читает `ibcmd`, и
/// запись остаётся его.
#[test]
fn reset_rewrites_the_record_with_the_tool_that_made_it() {
    let project = Project::with_sets(&[("main", "CONFIGURATION", "sources")], "  push: ibcmd\n");
    let config = fs::read_to_string(&project.config).expect("config");
    fs::write(&project.config, config.replace("  push: designer\n", "")).expect("config");
    succeeded(&project.run(&["push", "--force"]));
    project.edit();
    succeeded(&project.run(&["push", "--no-apply"]));
    assert_eq!(project.record()["tool"], "ibcmd", "{}", project.record());
    fs::write(project.state("ibcmd-rollback-token"), format!("{THIRD}\n")).expect("token");
    project.forget_calls();

    let reset = succeeded(&project.run(&["reset"]));

    assert_eq!(reset["data"]["provider"]["selected"], "designer", "{reset}");
    assert_eq!(reset["data"]["generation"], "recorded", "{reset}");
    let calls = project.calls();
    let rollback = calls.find("/RollbackCfg").expect("rollback");
    let read = calls
        .rfind("config generation-id")
        .expect("generation read");
    assert!(rollback < read, "{calls}");
    assert!(!calls.contains("/GetConfigGenerationID"), "{calls}");
    assert_eq!(project.record()["tool"], "ibcmd");
    assert_eq!(project.record()["token"], THIRD);
    assert_eq!(project.record()["after"], "build");
    assert!(
        project.record().get("applied").is_none(),
        "{}",
        project.record()
    );
}

/// Инструмент записи не ответил поколением после отката: запись стёрта и названа, а
/// следующая отправка не отказывает `no_memory` — память набора есть, пустая.
#[test]
fn reset_without_an_answer_erases_the_record() {
    let project = Project::new();
    succeeded(&project.run(&["push", "--force"]));
    project.edit();
    succeeded(&project.run(&["push", "--no-apply"]));
    fs::remove_file(project.state("token")).expect("no answer");

    let reset = succeeded(&project.run(&["reset"]));

    assert_eq!(reset["data"]["generation"], "erased", "{reset}");
    assert!(
        reset["data"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("its record is erased"),
        "{reset}"
    );
    assert!(project.record().is_null(), "{}", project.ledger());

    project.generation(FIRST);
    let pushed = succeeded(&project.run(&["push"]));
    assert_ne!(pushed["data"]["steps"][0]["mode"], "skipped", "{pushed}");
}

/// Без памяти о базе `reset` откатывает, но памяти не создаёт: ни хеш-памяти, ни записи, и
/// следующая отправка по-прежнему отказывает `no_memory`.
#[test]
fn reset_creates_no_memory_of_the_base() {
    let project = Project::new();
    fs::write(project.state("main"), "loaded elsewhere").expect("state");

    let reset = succeeded(&project.run(&["reset"]));

    assert_eq!(reset["data"]["outcome"], "discarded", "{reset}");
    assert_eq!(reset["data"]["hash_memory"], "absent", "{reset}");
    assert_eq!(reset["data"]["generation"], "unchecked", "{reset}");
    assert!(project.hashes().is_none());
    assert!(project.ledger().is_null(), "{}", project.ledger());

    let pushed = refused(&project.run(&["push"]));
    assert_eq!(pushed["error"]["kind"], "no_memory", "{pushed}");
}

/// Память другой пары «база ↔ каталог» — набор переехал в другой каталог — не своя: `reset`
/// её не трогает и запись чужой пары не переписывает.
#[test]
fn reset_leaves_the_memory_of_another_pair() {
    let project = Project::new();
    succeeded(&project.run(&["push", "--force"]));
    let moved = project.root().join("moved");
    fs::create_dir_all(&moved).expect("moved");
    for file in ["Configuration.xml", "Module.bsl"] {
        fs::copy(project.root().join("sources").join(file), moved.join(file)).expect("copy");
    }
    let config = fs::read_to_string(&project.config).expect("config");
    fs::write(
        &project.config,
        config.replace("path: sources", "path: moved"),
    )
    .expect("config");
    fs::write(project.state("main"), "loaded elsewhere").expect("state");
    let hashes = project.hashes();
    let ledger = project.ledger();

    let reset = succeeded(&project.run(&["reset"]));

    assert_eq!(reset["data"]["outcome"], "discarded", "{reset}");
    assert_eq!(reset["data"]["hash_memory"], "absent", "{reset}");
    assert_eq!(reset["data"]["generation"], "unchecked", "{reset}");
    assert!(hashes.is_some());
    assert_eq!(project.hashes(), hashes);
    assert_eq!(project.ledger(), ledger);
}

/// Превью ничего не читает и не запускает.
#[test]
fn reset_preview_dispatches_nothing() {
    let project = Project::new();
    fs::write(project.state("main"), "loaded").expect("state");

    let reset = succeeded(&project.run(&["--dry-run", "reset"]));

    assert_eq!(reset["data"]["outcome"], "planned", "{reset}");
    assert_eq!(reset["data"]["provider_dispatched"], false, "{reset}");
    assert!(project.calls().is_empty(), "{}", project.calls());
    assert_eq!(
        fs::read_to_string(project.state("main")).expect("main"),
        "loaded"
    );
}

/// `providers.reset` назначает исполнителя отката: `ibcmd` сохраняет конфигурации для
/// признака и откатывает `config reset`.
#[test]
fn providers_reset_assigns_the_executor_of_the_reset() {
    let project = Project::with_sets(&[("main", "CONFIGURATION", "sources")], "  reset: ibcmd\n");
    fs::write(project.state("main"), "loaded").expect("state");

    let reset = succeeded(&project.run(&["reset"]));

    assert_eq!(reset["data"]["provider"]["selected"], "ibcmd", "{reset}");
    assert_eq!(
        reset["data"]["provider"]["origin"]["kind"], "override",
        "{reset}"
    );
    assert_eq!(reset["data"]["outcome"], "discarded", "{reset}");
    let calls = project.calls();
    assert!(
        calls.contains("ibcmd infobase") && calls.contains("config save --db"),
        "{calls}"
    );
    let rolled = rollbacks(&calls);
    assert_eq!(rolled.len(), 1, "{calls}");
    assert!(rolled[0].starts_with("ibcmd "), "{calls}");
    assert!(!calls.contains("designer"), "{calls}");
    assert_eq!(
        fs::read_to_string(project.state("main")).expect("main"),
        "applied"
    );
}

/// Учётные данные базы уходят откату обоих исполнителей, а ответ их не называет.
#[test]
fn reset_passes_the_credentials_and_prints_no_secret() {
    for executor in ["designer", "ibcmd"] {
        let project = Project::with_sets(
            &[("main", "CONFIGURATION", "sources")],
            &format!("  reset: {executor}\n"),
        );
        fs::write(
            project.root().join("v8project.local.yaml"),
            "infobases:\n  origin:\n    connection: 'File=ib'\n    user: Admin\n    password: 'very-secret'\n",
        )
        .expect("local layer");
        fs::write(project.state("main"), "loaded").expect("state");

        let output = project.run(&["reset"]);
        let reset = succeeded(&output);

        assert_eq!(reset["data"]["outcome"], "discarded", "{executor}: {reset}");
        let calls = project.calls();
        let rolled = rollbacks(&calls);
        assert_eq!(rolled.len(), 1, "{executor}: {calls}");
        assert!(rolled[0].contains("Admin"), "{executor}: {calls}");
        assert!(rolled[0].contains("very-secret"), "{executor}: {calls}");
        let printed = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!printed.contains("very-secret"), "{executor}: {printed}");
    }
}

/// Формат EDT: память набора о базе — его копия Конфигуратора под памятью базы. `reset`
/// заменяет её пустой, и следующая отправка грузит копию заново, хотя исходники EDT с прошлого
/// перевода не менялись.
#[test]
fn an_edt_push_after_reset_loads_the_discarded_set_again() {
    let project = Project::new();
    let root = project.root();
    let edt_project = root.join("edt").join("configuration");
    fs::create_dir_all(edt_project.join("DT-INF")).expect("dt-inf");
    fs::create_dir_all(edt_project.join("src").join("Configuration")).expect("src");
    fs::write(
        edt_project.join(".project"),
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<projectDescription>\n  <name>configuration</name>\n  <natures>\n    <nature>com._1c.g5.v8.dt.core.V8ConfigurationNature</nature>\n  </natures>\n</projectDescription>\n",
    )
    .expect("project");
    fs::write(
        edt_project.join("DT-INF").join("PROJECT.PMF"),
        "Manifest-Version: 1.0\nRuntime-Version: 8.3.27\n",
    )
    .expect("manifest");
    let module = edt_project
        .join("src")
        .join("Configuration")
        .join("Module.bsl");
    fs::write(
        edt_project
            .join("src")
            .join("Configuration")
            .join("Configuration.mdo"),
        "<Configuration />\n",
    )
    .expect("mdo");
    fs::write(&module, "Procedure Test()\nEndProcedure\n").expect("module");
    let edt = root.join("edt-bin").join("1cedtcli");
    write_shell_script(
        &edt,
        &format!(
            "printf 'edt %s\\n' \"$*\" >> '{calls}'\ntarget=''\nprev=''\nfor arg in \"$@\"; do\n  if [ \"$prev\" = '--configuration-files' ]; then target=\"$arg\"; fi\n  prev=\"$arg\"\ndone\nif [ -n \"$target\" ]; then mkdir -p \"$target\"; printf '<Configuration />\\n' > \"$target/Configuration.xml\"; cp '{module}' \"$target/Module.bsl\"; fi\nexit 0",
            calls = root.join("calls.log").display(),
            module = module.display(),
        ),
    );
    let leads = support::DESIGNER_LEADS.replace("providers:\n", "");
    fs::write(
        &project.config,
        format!(
            "workPath: work\nformat: EDT\nproviders:\n{leads}source-set:\n  - name: main\n    type: CONFIGURATION\n    path: edt/configuration\ntools:\n  platform:\n    path: '{}'\n  edt_cli:\n    path: '{}'\n",
            project.bin.join("1cv8").display(),
            edt.display(),
        ),
    )
    .expect("config");
    succeeded(&project.run(&["push", "--force"]));
    fs::write(&module, "Procedure Test()\n// edited\nEndProcedure\n").expect("edit");
    let pushed = succeeded(&project.run(&["push", "--no-apply"]));
    assert_eq!(pushed["data"]["steps"][0]["applied"], false, "{pushed}");

    let reset = succeeded(&project.run(&["reset"]));
    assert_eq!(reset["data"]["outcome"], "discarded", "{reset}");
    assert_eq!(reset["data"]["hash_memory"], "replaced", "{reset}");

    project.forget_calls();
    let pushed = succeeded(&project.run(&["push"]));
    let loaded = pushed["data"]["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .any(|step| step["mode"] != "skipped" && step["applied"] == true);
    assert!(loaded, "{pushed}");
    assert!(
        project.calls().contains("/LoadConfigFromFiles"),
        "{}",
        project.calls()
    );
}

/// База ушла от записи до отката — кто-то загрузил и применил своё: откат идёт, а запись
/// остаётся прежней (`kept`), и следующая отправка по-прежнему отказывает `non_fast_forward`.
#[test]
fn reset_into_a_base_that_moved_keeps_the_record_and_the_next_push_is_refused() {
    let project = Project::new();
    succeeded(&project.run(&["push", "--force"]));
    project.edit();
    succeeded(&project.run(&["push", "--no-apply"]));
    project.generation(THIRD);
    let ledger = project.ledger();

    let reset = succeeded(&project.run(&["reset"]));

    assert_eq!(reset["data"]["outcome"], "discarded", "{reset}");
    assert_eq!(reset["data"]["generation"], "kept", "{reset}");
    assert!(
        reset["data"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("moved ahead of the record"),
        "{reset}"
    );
    assert_eq!(project.ledger(), ledger);

    let pushed = refused(&project.run(&["push"]));
    assert_eq!(pushed["error"]["kind"], "non_fast_forward", "{pushed}");
}

/// Запись, сделанная перед неудачной загрузкой, после отката остаётся как есть: её сверку
/// делает следующая отправка.
#[test]
fn reset_keeps_a_record_made_before_a_failed_load() {
    let project = Project::new();
    succeeded(&project.run(&["push", "--force"]));
    let file = project.memory().join("generation.json");
    let text = fs::read_to_string(&file).expect("ledger");
    fs::write(&file, text.replace("\"build\"", "\"failed_build\"")).expect("ledger");
    assert_eq!(project.record()["after"], "failed_build");
    fs::write(project.state("main"), "half loaded").expect("state");
    let ledger = project.ledger();
    project.forget_calls();

    let reset = succeeded(&project.run(&["reset"]));

    assert_eq!(reset["data"]["outcome"], "discarded", "{reset}");
    assert_eq!(reset["data"]["generation"], "kept", "{reset}");
    assert_eq!(project.ledger(), ledger);
    assert!(
        !project.calls().contains("/GetConfigGenerationID"),
        "{}",
        project.calls()
    );
}

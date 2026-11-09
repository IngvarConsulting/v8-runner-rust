//! Минимум платформы и граница `restore-ib` агента, как их видит вызывающий.
//!
//! Поддельная установка — каталог версии с файлом утилиты; все проверки идут превью или
//! отказом до запуска, поэтому файл никогда не исполняется и набор идёт и под Windows.

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::Value;
use support::{temp_workspace, v8_runner_command};

/// Проект с файловой базой и платформой `version`; `extra` — дополнительные ключи конфига.
fn project(dir: &Path, version: &str, extra: &str) -> PathBuf {
    let install = dir.join("platforms").join(version);
    let bin = install.join("bin");
    fs::create_dir_all(&bin).expect("bin");
    let binary = bin.join(if cfg!(windows) { "1cv8.exe" } else { "1cv8" });
    fs::write(
        &binary,
        format!("#!/bin/sh\ntouch '{}'\n", dir.join("ran").display()),
    )
    .expect("1cv8");
    #[cfg(unix)]
    support::make_executable(&binary);

    let sources = dir.join("src");
    fs::create_dir_all(&sources).expect("src");
    fs::write(sources.join("Configuration.xml"), "<Configuration/>\n").expect("Configuration.xml");
    let infobase = dir.join("build").join("ib");
    fs::create_dir_all(&infobase).expect("infobase");
    fs::write(infobase.join("1Cv8.1CD"), "base").expect("1Cv8.1CD");
    fs::write(dir.join("in.dt"), "dt").expect("dt");

    let config = dir.join("v8project.yaml");
    fs::write(
        &config,
        format!(
            "workPath: work\nformat: DESIGNER\ninfobase:\n  connection: \"File=build/ib\"\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\ntools:\n  platform:\n    path: '{}'\n{extra}",
            install.display()
        ),
    )
    .expect("config");
    config
}

fn run(config: &Path, args: &[&str]) -> Output {
    v8_runner_command()
        .arg("--config")
        .arg(config)
        .arg("--json-message")
        .args(args)
        .output()
        .expect("run")
}

fn envelope(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "not an envelope: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn restore_preview(config: &Path, dir: &Path) -> Output {
    run(
        config,
        &[
            "infobase",
            "restore",
            "--input",
            &dir.join("in.dt").display().to_string(),
            "--replace",
            "--dry-run",
        ],
    )
}

fn dump_preview(config: &Path, dir: &Path) -> Output {
    run(
        config,
        &[
            "infobase",
            "dump",
            "--output",
            &dir.join("out.dt").display().to_string(),
            "--dry-run",
        ],
    )
}

/// 8.3.23 отказывает родом `environment`, называя версию и минимум, и не запускается; 8.3.24
/// принимается той же командой.
#[test]
fn a_platform_below_the_minimum_is_refused_before_it_starts() {
    let dir = temp_workspace();
    let config = project(dir.path(), "8.3.23.9999", "");

    let output = dump_preview(&config, dir.path());

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let answer = envelope(&output);
    assert_eq!(
        answer["error"]["code"], "environment_unavailable",
        "{answer}"
    );
    let message = answer["error"]["message"].as_str().expect("message");
    assert!(message.contains("8.3.23.9999"), "{message}");
    assert!(message.contains("platform 8.3.24 and newer"), "{message}");
    assert!(!dir.path().join("ran").exists(), "the platform was started");

    let dir = temp_workspace();
    let config = project(dir.path(), "8.3.24.1819", "");
    let output = dump_preview(&config, dir.path());
    assert!(output.status.success(), "{output:?}");
    assert_eq!(envelope(&output)["ok"], true);
}

/// `platforms` — диагностика: установка ниже минимума названа, а выбрать её раннер не даст, и
/// причина в `warnings`.
#[test]
fn platforms_name_an_installation_below_the_minimum_and_do_not_select_it() {
    let dir = temp_workspace();
    let config = project(dir.path(), "8.3.23.9999", "");

    let output = run(&config, &["platforms"]);

    assert!(output.status.success(), "{output:?}");
    let answer = envelope(&output);
    assert_eq!(
        answer["data"]["installations"][0]["version"], "8.3.23.9999",
        "{answer}"
    );
    assert_eq!(
        answer["data"]["selected"]["designer"],
        Value::Null,
        "{answer}"
    );
    assert!(
        answer["warnings"]
            .as_array()
            .expect("warnings")
            .iter()
            .any(|warning| warning
                .as_str()
                .is_some_and(|w| w.contains("platform 8.3.24 and newer"))),
        "{answer}"
    );
}

/// Ниже 8.3.27 агент `restore-ib` спрашивает разрешение: цепочка идёт к Конфигуратору и
/// называет причину; с 8.3.27 первым остаётся агент.
#[test]
fn a_restore_below_8_3_27_takes_the_designer_and_from_8_3_27_the_agent() {
    for (version, selected) in [
        ("8.3.24.1819", "designer"),
        ("8.3.26.1656", "designer"),
        ("8.3.27.2342", "agent"),
        ("8.5.1.1529", "agent"),
    ] {
        let dir = temp_workspace();
        let config = project(dir.path(), version, "");

        let output = restore_preview(&config, dir.path());

        assert!(output.status.success(), "{version}: {output:?}");
        let answer = envelope(&output);
        let provider = &answer["data"]["provider"];
        assert_eq!(provider["selected"], selected, "{version}: {answer}");
        let skipped = provider["skipped"].as_array();
        if selected == "designer" {
            let skip = &skipped.expect("skipped")[0];
            assert_eq!(skip["provider"], "agent", "{answer}");
            let reason = skip["reason"].as_str().expect("reason");
            assert!(reason.contains(version), "{reason}");
            assert!(reason.contains("restore-ib"), "{reason}");
        } else {
            assert!(skipped.is_none_or(Vec::is_empty), "{version}: {answer}");
        }
        assert!(!dir.path().join("ran").exists(), "the platform was started");
    }
}

/// Граница касается только `restore-ib`: снимок ниже 8.3.27 агент снимает как прежде.
#[test]
fn a_dump_below_8_3_27_stays_with_the_agent() {
    let dir = temp_workspace();
    let config = project(dir.path(), "8.3.26.1656", "");

    let output = dump_preview(&config, dir.path());

    assert!(output.status.success(), "{output:?}");
    let answer = envelope(&output);
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(answer["data"]["provider"]["selected"], "agent", "{answer}");
}

/// Ключ, назначивший агента, не подменяется Конфигуратором: команда отказывает до запуска и
/// называет причину.
#[test]
fn an_assigned_agent_below_8_3_27_refuses_the_restore_with_the_reason() {
    let dir = temp_workspace();
    let config = project(
        dir.path(),
        "8.3.26.1656",
        "providers:\n  infobase.restore: agent\n",
    );

    let output = restore_preview(&config, dir.path());

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let answer = envelope(&output);
    assert_eq!(
        answer["error"]["code"], "environment_unavailable",
        "{answer}"
    );
    let message = answer["error"]["message"].as_str().expect("message");
    assert!(message.contains("restore-ib"), "{message}");
    assert!(message.contains("Designer"), "{message}");
    assert!(!dir.path().join("ran").exists(), "the platform was started");
}

/// Команда, которая только спрашивает базу, отказом родом `environment` не отвечает, но причину
/// называет: `status --deep` — в ответе о базе, `push` без памяти — в `skipped`. Платформа не
/// запускается ни там, ни там.
#[test]
fn status_deep_and_a_push_without_memory_name_the_minimum_as_a_reason() {
    let dir = temp_workspace();
    let config = project(dir.path(), "8.3.23.9999", "");

    let output = run(&config, &["status", "--deep"]);
    assert!(output.status.success(), "{output:?}");
    let answer = envelope(&output);
    let reason = answer["data"]["infobases"][0]["source_sets"][0]["base"]["reason"]
        .as_str()
        .expect("reason");
    assert!(reason.contains("platform 8.3.24 and newer"), "{answer}");

    let output = run(&config, &["push"]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    let answer = envelope(&output);
    assert_eq!(answer["error"]["code"], "untracked_base", "{answer}");
    let skipped = answer["data"]["provider"]["skipped"]
        .as_array()
        .expect("skipped");
    assert!(
        skipped.iter().any(|skip| skip["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("platform 8.3.24 and newer"))),
        "{answer}"
    );

    assert!(!dir.path().join("ran").exists(), "the platform was started");
}

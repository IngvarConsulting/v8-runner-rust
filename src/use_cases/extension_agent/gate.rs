//! Проверка синонима по языкам рабочей конфигурации в подключённой сессии шлюза.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use regex::Regex;

use super::{names_a_language, NO_SHIPPED_LANGUAGE};
use crate::platform::agent::{AgentError, WaitPolicy};
use crate::platform::extension_inventory::{is_extension_identifier, is_windows_device_name};
use crate::platform::sftp::FX_FAILURE;
use crate::support::error::AppError;
use crate::support::source_descriptor::{designer_child_objects, designer_language_code};
use crate::use_cases::agent_session::{
    make_output_dir, run_command, run_id, tidy, write_text, AgentHandle, Exchange,
};

/// Quoting measured in the ibsrv shell, where a quote preceded by backslashes
/// consumes each pair of backslashes and uses the odd one to escape the quote.
pub(super) fn shell_argument(value: &str) -> String {
    let quoted = value.chars().any(char::is_whitespace);
    let mut argument = String::with_capacity(value.len());
    if quoted {
        argument.push('"');
    }
    let mut backslashes = 0;
    for character in value.chars() {
        if character == '"' {
            argument.extend(std::iter::repeat_n('\\', backslashes + 1));
        }
        argument.push(character);
        backslashes = if character == '\\' {
            backslashes + 1
        } else {
            0
        };
    }
    if quoted {
        argument.extend(std::iter::repeat_n('\\', backslashes));
        argument.push('"');
    }
    argument
}

pub(super) fn prepare_synonym(
    handle: &mut AgentHandle,
    exchange: &Exchange,
    wait: &WaitPolicy,
    value: &str,
) -> Result<String, AppError> {
    let explicit = named_languages(value)?;
    wait.refuse_if_cancelled("read configuration languages")?;
    let scratch = format!("extension-languages/{}", run_id());
    let codes = configuration_languages(handle, exchange, wait, &scratch);
    tidy(handle, exchange, &scratch);
    let codes =
        codes.map_err(|error| error.with_context("read SSH gate configuration languages"))?;

    if let Some(explicit) = explicit {
        if let Some(absent) = explicit.into_iter().find(|code| !codes.contains(*code)) {
            return Err(AppError::Validation(format!(
                "the configuration has no language with code '{absent}' in --synonym"
            )));
        }
        return Ok(value.to_owned());
    }
    let escaped = value.replace('\'', "''");
    let form = ["ru", "en"]
        .into_iter()
        .filter(|code| codes.contains(*code))
        .map(|code| format!("{code}='{escaped}'"))
        .collect::<Vec<_>>()
        .join("; ");
    if form.is_empty() {
        return Err(AppError::Validation(NO_SHIPPED_LANGUAGE.to_owned()));
    }
    Ok(form)
}

fn named_languages(value: &str) -> Result<Option<Vec<&str>>, AppError> {
    // A separator belongs to the form only after the closing apostrophe. Two
    // apostrophes inside the literal encode one apostrophe.
    static ENTRY: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^([\p{Alphabetic}\p{Number}_]+)\s*=\s*'(?:[^']|'')*'")
            .expect("constant NStr entry pattern")
    });
    let mut rest = value.trim_start();
    let mut codes = Vec::new();
    while let Some(entry) = ENTRY.captures(rest) {
        let (matched, [code]) = entry.extract();
        codes.push(code);
        rest = rest[matched.len()..].trim_start();
        if rest.is_empty() {
            return Ok(Some(codes));
        }
        match rest.strip_prefix(';').map(str::trim_start) {
            Some("") => return Ok(Some(codes)),
            Some(next) => rest = next,
            None => break,
        }
    }
    if codes.is_empty() && !names_a_language(value) {
        return Ok(None);
    }
    Err(AppError::Validation(
        "invalid --synonym: expected language='text' entries separated by semicolons".to_owned(),
    ))
}

fn configuration_languages(
    handle: &mut AgentHandle,
    exchange: &Exchange,
    wait: &WaitPolicy,
    scratch: &str,
) -> Result<BTreeSet<String>, AppError> {
    let output = format!("{scratch}/xml");
    make_output_dir(handle, exchange, &output)?;
    let config_selector = format!("{scratch}/configuration.txt");
    let mut dump = format!("config dump-config-to-files --dir={output}");
    let selected = match exchange {
        Exchange::Dir(_) => {
            write_text(handle, exchange, &config_selector, "Configuration\n")?;
            true
        }
        Exchange::Sftp => match handle
            .ssh()?
            .sftp_write(&config_selector, b"Configuration\n")
        {
            Ok(()) => true,
            // Older ibsrv gateways reject opening files for write with this status.
            // A full server-side export needs no uploaded selector.
            Err(AgentError::SftpStatus {
                code: FX_FAILURE, ..
            }) => false,
            Err(error) => return Err(error.into()),
        },
    };
    let command = if selected {
        format!("{dump} --list-file={config_selector}")
    } else {
        dump.clone()
    };
    run_command(handle, &command, wait)?;
    wait.refuse_if_cancelled("read configuration languages")?;
    let config = read_xml(handle, exchange, &format!("{output}/Configuration.xml"))?;
    let children = designer_child_objects(&config).ok_or_else(|| {
        AppError::InvalidOutput("Configuration.xml has no readable ChildObjects".to_owned())
    })?;
    let mut names = BTreeSet::new();
    for (_, name) in children.into_iter().filter(|(kind, _)| kind == "Language") {
        if !is_extension_identifier(&name)
            || (cfg!(windows)
                && matches!(exchange, Exchange::Dir(_))
                && is_windows_device_name(&name))
        {
            return Err(AppError::InvalidOutput(format!(
                "unusable language object name '{name}' in Configuration.xml"
            )));
        }
        if !names.insert(name) {
            return Err(AppError::InvalidOutput(
                "Configuration.xml declares a language object twice".to_owned(),
            ));
        }
    }
    if selected && !names.is_empty() {
        wait.refuse_if_cancelled("read configuration languages")?;
        let language_selector = format!("{scratch}/languages.txt");
        let selectors: String = names
            .iter()
            .map(|name| format!("Language.{name}\n"))
            .collect();
        write_text(handle, exchange, &language_selector, &selectors)?;
        dump.push_str(&format!(" --list-file={language_selector}"));
        run_command(handle, &dump, wait)?;
    }
    names
        .iter()
        .map(|name| {
            wait.refuse_if_cancelled("read configuration languages")?;
            let xml = read_xml(handle, exchange, &format!("{output}/Languages/{name}.xml"))?;
            designer_language_code(&xml)
                .map_err(|error| {
                    AppError::InvalidOutput(format!("invalid XML for Language.{name}: {error}"))
                })?
                .filter(|code| !code.is_empty())
                .ok_or_else(|| {
                    AppError::InvalidOutput(format!("Language.{name} has no LanguageCode"))
                })
        })
        .collect()
}

fn read_xml(
    handle: &mut AgentHandle,
    exchange: &Exchange,
    relative: &str,
) -> Result<String, AppError> {
    let bytes = match exchange {
        Exchange::Dir(user_dir) => std::fs::read(user_dir.join(relative)).map_err(|error| {
            AppError::Platform(format!(
                "cannot read agent exchange file '{relative}': {error}"
            ))
        })?,
        Exchange::Sftp => handle.ssh()?.sftp_read(relative)?,
    };
    String::from_utf8(bytes)
        .map_err(|error| AppError::InvalidOutput(format!("invalid UTF-8 in '{relative}': {error}")))
}

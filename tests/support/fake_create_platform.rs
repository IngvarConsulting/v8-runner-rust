//! Поддельный Конфигуратор для `infobase create`: `CREATEINFOBASE File='<путь>'` заводит в
//! каталоге `1Cv8.1CD` с текстом `new` и `1Cv8.1CL`; при `V8_RUNNER_FAKE_CREATE_FAIL` — только
//! `1Cv8.1CD` с текстом `partial` и код выхода 1. `/UseTemplate` и `/RestoreIB` копируют
//! байты входного файла в `1Cv8.1CD`. Остальные вызовы отвечают нулём.

use std::path::PathBuf;

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if let Some(calls) = std::env::var_os("V8_RUNNER_FAKE_PLATFORM_CALLS") {
        use std::io::Write as _;
        let mut calls = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(calls)
            .expect("call log");
        writeln!(calls, "{arguments:?}").expect("record call");
    }
    let value_after = |key: &str| {
        arguments
            .windows(2)
            .find(|pair| pair[0].eq_ignore_ascii_case(key))
            .map(|pair| pair[1].as_str())
    };
    if let Some(image) = value_after("/RestoreIB") {
        let path = value_after("/F")
            .or_else(|| {
                value_after("/IBConnectionString")?
                    .strip_prefix("File=")?
                    .split(';')
                    .next()
                    .map(|path| path.trim_matches(['\'', '"']))
            })
            .expect("a file restore target");
        let dir = PathBuf::from(path);
        std::fs::create_dir_all(&dir).expect("infobase dir");
        std::fs::copy(image, dir.join("1Cv8.1CD")).expect("restore DT");
        return;
    }
    let Some(at) = arguments.iter().position(|arg| arg == "CREATEINFOBASE") else {
        return;
    };
    let connection = &arguments[at + 1];
    let path = connection
        .strip_prefix("File='")
        .and_then(|rest| rest.split('\'').next())
        .expect("a file connection");
    let dir = PathBuf::from(path);
    std::fs::create_dir_all(&dir).expect("infobase dir");
    if std::env::var_os("V8_RUNNER_FAKE_CREATE_FAIL").is_some() {
        std::fs::write(dir.join("1Cv8.1CD"), "partial").expect("partial base");
        eprintln!("fake create failed");
        std::process::exit(1);
    }
    if let Some(template) = value_after("/UseTemplate") {
        std::fs::copy(template, dir.join("1Cv8.1CD")).expect("load CF template");
    } else {
        std::fs::write(dir.join("1Cv8.1CD"), "new").expect("new base");
    }
    std::fs::write(dir.join("1Cv8.1CL"), "new log").expect("new log");
}

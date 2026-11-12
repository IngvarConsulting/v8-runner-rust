//! Поддельный `ibcmd` для всех ОС: собирается `rustc` в тесте. Пишет свои ключи в
//! `ibcmd-args.log` каталога `V8R_FAKE_DESIGNER_DIR` и на `config extension list` отвечает
//! записью расширения `Зонд` — той же, что двойник агента (`support::fake_agent`) по
//! умолчанию отдаёт на `config extensions properties get --all-extensions`. Подтверждение
//! префикса раннером тоже проходит: `config save` пишет файл, а `config export --file=…` —
//! `Configuration.xml` применённого расширения с префиксом `Зо_`. Прочие команды отказывают
//! кодом 1.

use std::io::Write;
use std::path::PathBuf;

fn main() {
    let dir = PathBuf::from(std::env::var_os("V8R_FAKE_DESIGNER_DIR").expect("V8R_FAKE_DESIGNER_DIR"));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("ibcmd-args.log"))
        .expect("args log");
    writeln!(log, "{}", args.join(" ")).expect("args line");
    let has = |first: &str, second: &str| {
        args.windows(2)
            .any(|pair| pair[0] == first && pair[1] == second)
    };
    let last = args.last().cloned().unwrap_or_default();
    if has("config", "save") {
        std::fs::write(&last, "cfe").expect("saved extension");
        return;
    }
    if args.iter().any(|arg| arg == "export") && args.iter().any(|arg| arg.starts_with("--file=")) {
        std::fs::create_dir_all(&last).expect("export dir");
        std::fs::write(
            std::path::Path::new(&last).join("Configuration.xml"),
            "<MetaDataObject><Configuration><Properties><Name>Зонд</Name><ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose><NamePrefix>Зо_</NamePrefix></Properties></Configuration></MetaDataObject>",
        )
        .expect("descriptor");
        return;
    }
    if !has("extension", "list") {
        std::process::exit(1);
    }
    print!(
        "name                         : \"Зонд\"\nversion                      : \nactive                       : yes\npurpose                      : customization\nsafe-mode                    : yes\nsecurity-profile-name        : \nunsafe-action-protection     : yes\nused-in-distributed-infobase : no\nscope                        : infobase\nhash-sum                     : \"8\"\n"
    );
}

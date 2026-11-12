//! Поддельный `ibcmd` для всех ОС: собирается `rustc` в тесте. Пишет свои ключи в
//! `ibcmd-args.log` каталога `V8R_FAKE_DESIGNER_DIR` и на `config extension list` отвечает
//! записью расширения `Зонд` — той же, что двойник агента (`support::fake_agent`) по
//! умолчанию отдаёт на `config extensions properties get --all-extensions`. Прочие команды
//! отказывают кодом 1.

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
    let lists = args.windows(2).any(|pair| pair[0] == "extension" && pair[1] == "list");
    if !lists {
        std::process::exit(1);
    }
    print!(
        "name                         : \"Зонд\"\nversion                      : \nactive                       : yes\npurpose                      : customization\nsafe-mode                    : yes\nsecurity-profile-name        : \nunsafe-action-protection     : yes\nused-in-distributed-infobase : no\nscope                        : infobase\nhash-sum                     : \"8\"\n"
    );
}

//! Поддельный `1cv8` в агентском режиме для всех ОС: собирается `rustc` в тесте. Делает то же,
//! что сценарий оболочки `fake_agent::write_fake_designer`: пишет ключи запуска и свой pid,
//! раскладку `AgentBaseDir`, как платформа, и файл запроса для двойника агента, затем живёт,
//! пока его не снимут. Куда писать, называет переменная `V8R_FAKE_DESIGNER_DIR`: в ней
//! `designer-args.log`, `designer.pid`, `base-dir.txt` и `base-dir.txt.launch`.

use std::io::Write;
use std::path::PathBuf;

fn main() {
    let dir = PathBuf::from(std::env::var_os("V8R_FAKE_DESIGNER_DIR").expect("V8R_FAKE_DESIGNER_DIR"));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("designer-args.log"))
        .expect("args log");
    writeln!(log, "{}", args.join(" ")).expect("args line");
    std::fs::write(dir.join("designer.pid"), std::process::id().to_string()).expect("pid");
    let value = |name: &str| {
        args.windows(2)
            .find(|pair| pair[0] == name)
            .map(|pair| pair[1].clone())
    };
    let Some(base) = value("/AgentBaseDir") else {
        std::process::exit(3);
    };
    let port = value("/AgentPort").unwrap_or_default();
    let key = value("/AgentSSHHostKey").unwrap_or_default();
    let base = PathBuf::from(base);
    std::fs::create_dir_all(base.join("0")).expect("user dir");
    std::fs::write(
        base.join("agentbasedir.json"),
        r#"{"usersInfo":[{"name":"","dir":"0"}]}"#,
    )
    .expect("agent map");
    let launch = dir.join("base-dir.txt.launch");
    let staged = dir.join("base-dir.txt.launch.tmp");
    std::fs::write(&staged, format!("{port}\n{key}\n")).expect("launch request");
    std::fs::rename(&staged, &launch).expect("publish launch request");
    std::fs::write(dir.join("base-dir.txt"), base.display().to_string()).expect("base dir");
    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

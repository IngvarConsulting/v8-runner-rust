//! Листья с превью и образец проекта, на котором их превью прогоняются.
//!
//! Вызов каждого листа записан здесь один раз: его берут и страж следов
//! (`tests/contract_previews.rs`), и сверка форм `data` (`tests/contract_command_data.rs`).
//! Каждая из них сверяет состав с `LEAVES_WITH_PREVIEW` из `src/cli/global_flags_expected.in`,
//! поэтому лист, получивший превью без строки здесь, роняет обе проверки, а не проходит
//! мимо одной из них молча (#266, #268).

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{v8_runner_command, write_shell_script};

/// Образец проекта в `dir`: набор исходников, расширение инструмента и, если
/// `with_platform`, поддельные утилиты платформы и EDT CLI. Возвращает путь к
/// `v8project.yaml`.
pub fn write_project(dir: &Path, with_platform: bool) -> PathBuf {
    let base_path = dir.join("project");
    let work_path = dir.join("work");
    let install_dir = dir.join("platform");
    let extension_source = base_path.join("exts").join("client-mcp");
    fs::create_dir_all(&extension_source).expect("extension dir");
    // Расширение инструмента объявлено намеренно: шаг его подготовки — место, где превью
    // сборки однажды запускало платформу и писало состояние (#252). Без него образец
    // этого класса не видит.
    fs::write(
        extension_source.join("Configuration.xml"),
        "<Configuration><Properties><Name>client_mcp</Name><ConfigurationExtensionPurpose kind=\"Customization\">Customization</ConfigurationExtensionPurpose></Properties></Configuration>",
    )
    .expect("extension marker");
    fs::write(
        extension_source.join("Module.bsl"),
        "procedure Tool() endprocedure",
    )
    .expect("extension module");
    fs::create_dir_all(base_path.join("configuration")).expect("configuration dir");
    fs::write(
        base_path.join("configuration").join("Configuration.xml"),
        "<MetaDataObject/>",
    )
    .expect("configuration marker");
    fs::create_dir_all(&work_path).expect("work dir");
    // Веб-сервер объявлен, чтобы превью `publish` доходило до плана своей формой, а не
    // до общей формы отказа: сверка форм `data` требует от каждого листа формы его команды.
    let www = dir.join("www");
    fs::create_dir_all(&www).expect("www dir");
    fs::create_dir_all(install_dir.join("bin")).expect("platform dir");
    if with_platform {
        write_shell_script(&install_dir.join("bin").join("1cv8"), "exit 0");
        write_shell_script(&install_dir.join("bin").join("ibcmd"), "exit 0");
        write_shell_script(&install_dir.join("bin").join("webinst"), "exit 0");
        write_shell_script(&install_dir.join("bin").join("1cedtcli"), "exit 0");
    }

    // Без платформы поиск обязан отказать, а не уйти в PATH или в корни по умолчанию:
    // строгий режим с версией не даёт локатору найти платформу за пределами каталога.
    // EDT CLI строгого режима не знает и ищется ещё в PATH и корнях по умолчанию, поэтому
    // `convert` в `SUCCEEDS_HERE` (`tests/contract_previews.rs`) нет: без стаба на машине с EDT его превью прошло бы.
    let strictness = if with_platform {
        ""
    } else {
        "    strict: true\n    version: '8.3.27'\n"
    };
    let config_path = dir.join("v8project.yaml");
    fs::write(
        &config_path,
        format!(
            "workPath: {}\nformat: DESIGNER\ninfobase:\n  connection: 'File={}'\n  web:\n    server: apache24\n    wsdir: demo\n    dir: '{}'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project/configuration\ntools:\n  platform:\n    path: {}\n{strictness}  edt_cli:\n    path: {}\n    interactive-mode: false\n  client_mcp:\n    extension:\n      name: client_mcp\n      source:\n        path: {}\n",
            work_path.display(),
            dir.join("ib").display(),
            www.display(),
            install_dir.display(),
            install_dir.join("bin").join("1cedtcli").display(),
            extension_source.display()
        ),
    )
    .expect("write config");
    config_path
}

/// Настройки берутся из текущего каталога, а не глобальным ключом: `clone` его отвергает,
/// а образцу он не нужен — `v8project.yaml` лежит в корне образца. Переменная `V8TR_CONFIG`
/// снимается вместе с ключом: она объявлена его умолчанием, и чужое окружение увело бы
/// весь образец в другой проект молча.
pub fn run(dir: &Path, arguments: &[String]) -> (i32, Value) {
    let output = v8_runner_command()
        .current_dir(dir)
        .env_remove("V8TR_CONFIG")
        .arg("--json-message")
        .args(arguments)
        .output()
        .expect("run command");
    let payload: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "`{}` printed no json envelope: {error}\nstdout: {}\nstderr: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status.code().unwrap_or(-1), payload)
}

/// Строка таблицы: вызов, путь листа, имя команды в ответе и путь, которого после превью
/// быть не должно.
pub struct Previewed {
    pub arguments: Vec<String>,
    pub leaf: &'static str,
    /// Имя, которым ответ обязан назвать команду в конверте: прежние и вложенные пути
    /// отвечают именем своей команды (`infobase configuration export` — `download`).
    pub command: &'static str,
    /// След, который превью оставило бы, если бы работало. У прочих листьев это
    /// общий рабочий каталог; у `clone` — каталог проекта, которого он ещё не завёл.
    pub trace: PathBuf,
}

fn row(arguments: &[&str], leaf: &'static str, command: &'static str, trace: PathBuf) -> Previewed {
    Previewed {
        arguments: arguments.iter().map(|value| (*value).to_owned()).collect(),
        leaf,
        command,
        trace,
    }
}

/// Каждый лист с превью: минимальный вызов, путь листа и имя его команды. Состав
/// сверяется с общим списком `LEAVES_WITH_PREVIEW` в каждом наборе, который таблицу берёт,
/// поэтому новый лист с превью обязан появиться и здесь.
pub fn with_preview(dir: &Path) -> Vec<Previewed> {
    let work = dir.join("work");
    let artifact = dir.join("main.cf").display().to_string();
    let snapshot = dir.join("main.dt").display().to_string();
    let cloned = dir.join("cloned");
    let platform = dir.join("platform").display().to_string();
    let source = format!("File={}", dir.join("ib").display());
    vec![
        // `clone` проектного файла не читает и глобальный ключ настроек отвергает: адрес,
        // версию и подсказку платформы он называет своими ключами, а писать будет в свой
        // каталог. След у него поэтому тоже свой.
        row(
            &[
                "clone",
                "--project-dir",
                &cloned.display().to_string(),
                "--connection",
                &source,
                "--platform-version",
                "8.3.27",
                "--platform-path",
                &platform,
            ],
            "clone",
            "clone",
            cloned,
        ),
        row(&["extensions"], "extensions", "extensions", work.clone()),
        row(
            &["extensions", "list"],
            "extensions list",
            "extensions",
            work.clone(),
        ),
        row(
            &["extensions", "info", "--name", "client_mcp"],
            "extensions info",
            "extensions",
            work.clone(),
        ),
        row(
            &[
                "extensions",
                "create",
                "--name",
                "Demo",
                "--name-prefix",
                "Demo",
            ],
            "extensions create",
            "extensions",
            work.clone(),
        ),
        row(
            &["extensions", "delete", "--name", "client_mcp"],
            "extensions delete",
            "extensions",
            work.clone(),
        ),
        row(
            &[
                "extensions",
                "activate",
                "--name",
                "client_mcp",
                "--active",
                "yes",
            ],
            "extensions activate",
            "extensions",
            work.clone(),
        ),
        row(&["build"], "push", "push", work.clone()),
        row(
            &["load", "--path", &artifact],
            "upload",
            "upload",
            work.clone(),
        ),
        row(&["dump", "--force"], "pull", "pull", work.clone()),
        row(
            &["download", "--state", "working", "--output", &artifact],
            "download",
            "download",
            work.clone(),
        ),
        row(
            &["infobase", "create"],
            "infobase create",
            "infobase create",
            work.clone(),
        ),
        row(
            &[
                "infobase",
                "configuration",
                "export",
                "--state",
                "working",
                "--output",
                &artifact,
            ],
            "infobase configuration export",
            "download",
            work.clone(),
        ),
        row(
            &["infobase", "dump", "--output", &snapshot],
            "infobase dump",
            "infobase.dump",
            work.clone(),
        ),
        row(
            &["infobase", "restore", "--input", &snapshot, "--replace"],
            "infobase restore",
            "infobase.restore",
            work.clone(),
        ),
        row(&["convert"], "convert", "convert", work.clone()),
        row(
            &["make", "--output", &artifact],
            "make",
            "make",
            work.clone(),
        ),
        row(&["check"], "check", "check", work.clone()),
        row(
            &["check", "designer-config"],
            "check designer-config",
            "check",
            work.clone(),
        ),
        row(
            &["check", "designer-modules", "--thin-client"],
            "check designer-modules",
            "check",
            work.clone(),
        ),
        row(&["check", "edt"], "check edt", "check", work.clone()),
        row(&["launch", "designer"], "launch", "launch", work.clone()),
        row(&["publish"], "publish", "publish", work),
    ]
}

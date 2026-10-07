//! Двойник агента Конфигуратора: SSH-сервер на `russh` внутри процесса теста.
//!
//! Настоящее рукопожатие и аутентификация по паролю, канал shell, ответы —
//! JSON-массивами в форме агента: долгие команды шлют прогресс и журнал отдельными
//! массивами и лишь потом итог (замер 15.09.2026). Файлы двойник читает и пишет в
//! каталоге пользователя агента — через ссылки, как настоящий агент.
#![allow(dead_code)]

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::server::{self, Auth, Msg, Server as _, Session};
use russh::{Channel, ChannelId};
use russh_sftp::protocol::{
    Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, Status, StatusCode, Version,
};

pub const AGENT_PASSWORD: &str = "agentpass";

/// Команда, которую двойник держит, пока тест её не отпустит: так тест прерывает раннер
/// посреди команды, не гадая по часам. Держится каждая подходящая команда каждого
/// соединения: двойник клонируется на соединение, и удержание едет с ним. Ждёт двойник
/// внутри обработчика данных, поэтому, пока команда удержана, сессия этого соединения не
/// обслуживает ничего другого — ни SFTP, ни keepalive.
#[derive(Clone)]
pub struct Hold {
    /// Начало строки команды.
    pub command: String,
    /// Файл, который двойник кладёт, когда команда пришла.
    pub started: PathBuf,
    /// Файл, которого двойник ждёт, прежде чем ответить.
    pub release: PathBuf,
    /// Чем двойник отвечает, когда его отпустили.
    pub reply: HoldReply,
}

/// Ответ удержанной команды.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HoldReply {
    /// Обычный ответ двойника на эту команду.
    Normal,
    /// Сообщение об ошибке агента.
    Error,
    /// Канал закрыт без итогового сообщения.
    Close,
    /// Команда исполнена как обычно, но ответ о ней — проза вместо массива сообщений; с
    /// `close` двойник после неё закрывает канал.
    Prose { text: &'static str, close: bool },
}

/// Запись по SFTP, которую двойник держит, пока тест её не отпустит. Держится только
/// первая запись соединения: SFTP-сервер забирает удержание, открывая её.
#[derive(Clone)]
pub struct SftpHold {
    /// Файл, который двойник кладёт, когда запись началась.
    pub started: PathBuf,
    /// Файл, которого двойник ждёт, прежде чем писать.
    pub release: PathBuf,
}

/// Ждёт файла `release` не дольше полуминуты, уступая рантайм двойника. Паника внутри
/// обработчика russh пропала бы молча, поэтому просроченное ожидание пишется в stderr, а
/// двойник отвечает как обычно: тест падает на своих сроках, с понятной причиной.
async fn wait_for_release(release: &Path) {
    for _ in 0..3_000 {
        if release.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    eprintln!(
        "fake agent: the test never released the held command ({})",
        release.display()
    );
}

/// Один экземпляр на соединение, общее состояние — через `Arc`.
#[derive(Clone)]
pub struct FakeAgent {
    pub accept_password: bool,
    pub commands_log: PathBuf,
    /// Каталог `AgentBaseDir`: у чужого агента известен заранее, у управляемого его
    /// сообщает поддельный `1cv8` через файл.
    pub base_dir: Option<PathBuf>,
    pub base_dir_file: PathBuf,
    pub designer_pid_file: PathBuf,
    /// Поколение основной конфигурации: растёт с каждой удачной её загрузкой.
    pub generation: Arc<AtomicU64>,
    /// Поколения расширений по имени: у каждого своё, как у платформы.
    pub extension_generations: Arc<Mutex<HashMap<String, u64>>>,
    /// Двойник шлюза автономного сервера: каталог пользователя задан прямо (у шлюза
    /// нет карты `agentbasedir.json`), логин — имя пользователя базы.
    pub gate: Option<(String, PathBuf)>,
    /// SFTP только на чтение — как у живого шлюза `ibsrv` 8.3.27 (замер 15.09.2026:
    /// mkdir/rmdir/get работают, open на запись — Failure).
    pub sftp_read_only: bool,
    /// Команда shell, которую двойник держит до знака теста.
    pub hold: Option<Hold>,
    /// Запись по SFTP, которую двойник держит до знака теста.
    pub sftp_hold: Option<SftpHold>,
    /// Принимать любой логин и пароль управляемого агента: так двойник обслуживает
    /// команды, чьи учётные данные называет сам тест (`clone --user --password`).
    pub any_credentials: bool,
    /// Выгрузка в файлы отвечает ошибкой агента, пока флаг поднят.
    pub fail_dump: Arc<AtomicBool>,
    /// Каналы соединения: подсистема SFTP забирает свой канал в поток.
    channels: Arc<Mutex<HashMap<ChannelId, Channel<Msg>>>>,
    /// Каналы, отданные SFTP: их байты — не команды shell.
    sftp_channels: Arc<Mutex<Vec<ChannelId>>>,
    /// Состав расширений базы: `properties get/set`, `create`, `delete`.
    pub extensions: Arc<Mutex<Vec<FakeExtension>>>,
    /// Что было собрано из каких xml: обратная выгрузка возвращает тот же описатель.
    external_sources: Arc<Mutex<HashMap<PathBuf, String>>>,
    buffers: Arc<Mutex<HashMap<ChannelId, Vec<u8>>>>,
}

#[derive(Clone, Debug)]
pub struct FakeExtension {
    pub name: String,
    pub active: bool,
    pub safe_mode: bool,
    pub unsafe_action_protection: bool,
    pub purpose: String,
}

impl FakeExtension {
    /// Запись шлюза автономного сервера 8.3.27: `active` и `version` переставлены
    /// (живой ответ 15.09.2026).
    fn gate_record(&self) -> String {
        format!(
            "{{\"body\":{{\"active\":\"\",\"hash-sum\":\"{:x}\",\"name\":\"{}\",\"purpose\":\"{}\",\"safe-mode\":{},\"scope\":\"infobase\",\"security-profile-name\":\"\",\"unsafe-action-protection\":{},\"used-in-distributed-infobase\":false,\"version\":{}}},\"type\":\"extension-properties\"}}",
            self.name.len(), self.name, self.purpose, self.safe_mode, self.unsafe_action_protection, self.active
        )
    }

    fn record(&self) -> String {
        let hash = format!("{:x}", self.name.len());
        format!(
            "{{\"type\":\"extension-properties\",\"body\":{{\"name\":\"{}\",\"version\":\"\",\"active\":{},\"purpose\":\"{}\",\"safe-mode\":{},\"security-profile-name\":\"\",\"unsafe-action-protection\":{},\"used-in-distributed-infobase\":false,\"scope\":\"infobase\",\"hash-sum\":\"{hash}\"}}}}",
            self.name, self.active, self.purpose, self.safe_mode, self.unsafe_action_protection
        )
    }
}

impl FakeAgent {
    pub fn new(
        accept_password: bool,
        commands_log: PathBuf,
        base_dir: Option<PathBuf>,
        base_dir_file: PathBuf,
        designer_pid_file: PathBuf,
    ) -> Self {
        Self {
            accept_password,
            commands_log,
            base_dir,
            base_dir_file,
            designer_pid_file,
            gate: None,
            sftp_read_only: false,
            hold: None,
            sftp_hold: None,
            any_credentials: false,
            fail_dump: Arc::new(AtomicBool::new(false)),
            channels: Arc::new(Mutex::new(HashMap::new())),
            sftp_channels: Arc::new(Mutex::new(Vec::new())),
            generation: Arc::new(AtomicU64::new(1)),
            extension_generations: Arc::new(Mutex::new(HashMap::new())),
            extensions: Arc::new(Mutex::new(vec![FakeExtension {
                name: "Зонд".to_owned(),
                active: true,
                safe_mode: true,
                unsafe_action_protection: true,
                purpose: "customization".to_owned(),
            }])),
            external_sources: Arc::new(Mutex::new(HashMap::new())),
            buffers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Файловый параметр агента: относительно каталога пользователя и — как у настоящего
    /// агента — ни одной символической ссылки на пути («Файл не обнаружен»).
    fn file_arg(&self, relative: &str) -> Result<PathBuf, String> {
        let user_dir = self.user_dir();
        let mut probe = user_dir.clone();
        for component in Path::new(relative).components() {
            probe.push(component);
            if fs::symlink_metadata(&probe)
                .map(|meta| meta.file_type().is_symlink())
                .unwrap_or(false)
            {
                return Err(format!(
                    "[{{\"type\":\"error\",\"error-type\":\"UnknownError\",\"message\":\"Файл не обнаружен '{}'\"}}]\n",
                    probe.display()
                ));
            }
        }
        Ok(user_dir.join(relative))
    }

    fn extension_reply(&self, name: Option<&str>) -> String {
        let extensions = self.extensions.lock().expect("extensions");
        let records = extensions
            .iter()
            .filter(|extension| name.is_none_or(|name| extension.name == name))
            .map(|extension| {
                if self.gate.is_some() {
                    extension.gate_record()
                } else {
                    extension.record()
                }
            })
            .collect::<Vec<_>>();
        if name.is_some() && records.is_empty() {
            return extension_not_found();
        }
        // Живой агент отвечает по-разному: на одно расширение — одним сообщением
        // `extension-properties` без `success`, на все — списком в `body` у `success`.
        if name.is_some() {
            return format!("[{}]\n", records.join(","));
        }
        format!(
            "[{{\"type\":\"success\",\"message\":\"\",\"body\":[{}]}}]\n",
            records.join(",")
        )
    }

    /// Шлюз автономного сервера: сессии от `user` с паролем, файлы — в `user_dir`.
    pub fn gate(commands_log: PathBuf, user: &str, user_dir: PathBuf) -> Self {
        let mut agent = Self::new(
            true,
            commands_log,
            None,
            PathBuf::from("/nonexistent/base-dir-file"),
            PathBuf::from("/nonexistent/designer.pid"),
        );
        agent.gate = Some((user.to_owned(), user_dir));
        agent
    }

    fn user_dir(&self) -> PathBuf {
        if let Some((_, user_dir)) = self.gate.as_ref() {
            return user_dir.clone();
        }
        let base = match self.base_dir.as_ref() {
            Some(base) => base.clone(),
            None => PathBuf::from(fs::read_to_string(&self.base_dir_file).unwrap_or_default()),
        };
        base.join("0")
    }

    fn log(&self, line: &str) {
        if let Ok(mut log) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.commands_log)
        {
            use std::io::Write;
            let _ = writeln!(log, "{line}");
        }
    }

    fn token(&self) -> String {
        format!("{:040x}", self.generation.load(Ordering::SeqCst))
    }

    /// Токен поколения основной конфигурации или расширения `extension`.
    fn token_of(&self, extension: Option<&str>) -> String {
        match extension {
            None => self.token(),
            Some(name) => {
                let generations = self.extension_generations.lock().expect("generations");
                format!("e{:039x}", generations.get(name).copied().unwrap_or(1))
            }
        }
    }

    /// Загрузка меняет поколение того, во что грузит.
    fn advance(&self, extension: Option<&str>) {
        match extension {
            None => {
                self.generation.fetch_add(1, Ordering::SeqCst);
            }
            Some(name) => {
                *self
                    .extension_generations
                    .lock()
                    .expect("generations")
                    .entry(name.to_owned())
                    .or_insert(1) += 1;
            }
        }
    }

    /// Ответ на одну команду и признак «сессия завершается».
    fn respond(&self, line: &str) -> (String, bool) {
        self.log(line);
        let words = tokens(line);
        let option = |name: &str| -> Option<String> {
            words
                .iter()
                .find_map(|word| word.strip_prefix(&format!("--{name}=")))
                .map(str::to_owned)
        };
        let has = |flag: &str| words.iter().any(|word| word == &format!("--{flag}"));
        if line.starts_with("options set")
            || line == "common connect-ib"
            || line == "common disconnect-ib"
        {
            return (success(), false);
        }
        if line.starts_with("config generation-id") {
            return (
                format!(
                    "[{{\"type\":\"success\",\"body\":\"{}\"}}]\n",
                    self.token_of(option("extension").as_deref())
                ),
                false,
            );
        }
        if line.starts_with("config dump-config-to-files") {
            if self.fail_dump.load(Ordering::SeqCst) {
                return (
                    "[{\"type\":\"error\",\"error-type\":\"ConfigFilesError\",\"message\":\"Выгрузка не выполнена\"}]\n".to_owned(),
                    false,
                );
            }
            let dir = option("dir").unwrap_or_default();
            let target = self.user_dir().join(&dir);
            fs::create_dir_all(&target).expect("agent output dir");
            fs::write(target.join("Configuration.xml"), "<Configuration/>\n").expect("dump file");
            // Как платформа: выгрузка целиком и по изменившемуся пишет файл версий.
            if option("list-file").is_none() {
                fs::write(
                    target.join("ConfigDumpInfo.xml"),
                    "<ConfigDumpInfo format=\"Hierarchical\" version=\"2.17\"/>\n",
                )
                .expect("dump info");
            }
            if has("update") {
                fs::write(target.join("updated.txt"), "updated").expect("update marker");
            }
            if let Some(list) = option("list-file") {
                let objects = fs::read_to_string(self.user_dir().join(list)).unwrap_or_default();
                self.log(&format!(
                    "list: {}",
                    objects.split_whitespace().collect::<Vec<_>>().join(";")
                ));
            }
            return (progress_then_success("Выгрузка конфигурации"), false);
        }
        if line.starts_with("config load-config-from-files") {
            let dir = option("dir").unwrap_or_default();
            let source = self.user_dir().join(&dir);
            if !source.join("Configuration.xml").is_file() {
                return (
                    "[{\"type\":\"error\",\"error-type\":\"ConfigFilesError\",\"message\":\"Каталог загрузки пуст\"}]\n".to_owned(),
                    false,
                );
            }
            if let Some(list) = option("list-file") {
                let entries = fs::read_to_string(self.user_dir().join(list)).unwrap_or_default();
                let entries = entries.trim_start_matches('\u{feff}');
                self.log(&format!(
                    "list: {}",
                    entries
                        .lines()
                        .map(str::trim)
                        .filter(|l| !l.is_empty())
                        .collect::<Vec<_>>()
                        .join(";")
                ));
            }
            self.advance(option("extension").as_deref());
            // Как платформа: с `--update-config-dump-info` файл версий в каталоге загрузки
            // переписывается; рядом с журналом команд (`<журнал>.version-files`) записано,
            // какой файл загрузка там застала.
            if has("update-config-dump-info") {
                let version_file = source.join("ConfigDumpInfo.xml");
                let found = fs::read_to_string(&version_file).unwrap_or_default();
                if let Ok(mut seen) = fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(self.commands_log.with_extension("version-files"))
                {
                    use std::io::Write;
                    let _ = writeln!(seen, "{}", found.trim());
                }
                fs::write(
                    &version_file,
                    format!(
                        "<ConfigDumpInfo version=\"2.17\" agent-load=\"{}\"/>\n",
                        self.token()
                    ),
                )
                .expect("agent version file");
            }
            return (progress_then_success("Загрузка конфигурации"), false);
        }
        if line.starts_with("config update-db-cfg") {
            // Шлюз автономного сервера вставляет в ответ уведомление `generation-id`
            // с новым токеном до итога команды (живой прогон 15.09.2026).
            if self.gate.is_some() {
                return (
                    format!(
                        "[{{\"type\":\"log\",\"message\":\"Принятие изменений...\"}}]\n[{{\"body\":\"{}\",\"type\":\"generation-id\"}}]\n[{{\"type\":\"log\",\"message\":\"Обновление конфигурации базы данных успешно завершено\"}}]\n[{{\"type\":\"success\"}}]\n",
                        self.token()
                    ),
                    false,
                );
            }
            return (
                "[{\"type\":\"log\",\"message\":\"Обработка структуры базы данных...\"}]\n[{\"type\":\"log\",\"message\":\"Принятие изменений...\"}]\n[{\"type\":\"success\",\"message\":\"\"}]\n".to_owned(),
                false,
            );
        }
        if line.starts_with("config dump-cfg") {
            let file = option("file").unwrap_or_default();
            let target = match self.file_arg(&file) {
                Ok(target) => target,
                Err(reply) => return (reply, false),
            };
            if !target.parent().is_some_and(Path::is_dir) {
                return (
                    format!(
                        "[{{\"type\":\"error\",\"error-type\":\"UnknownError\",\"message\":\"Файл не обнаружен '{}'\"}}]\n",
                        target.display()
                    ),
                    false,
                );
            }
            let body = match option("extension") {
                Some(extension) => format!("CFE:{extension}"),
                None => "CF:main".to_owned(),
            };
            fs::write(&target, body).expect("dump-cfg file");
            return (
                progress_then_success("Сохранение конфигурации в файл"),
                false,
            );
        }
        if line.starts_with("infobase-tools dump-ib") {
            let file = option("file").unwrap_or_default();
            let target = match self.file_arg(&file) {
                Ok(target) => target,
                Err(reply) => return (reply, false),
            };
            fs::write(&target, "DT").expect("dump-ib file");
            return (progress_then_success("Выгрузка информационной базы"), false);
        }
        if line.starts_with("infobase-tools restore-ib") {
            let file = option("file").unwrap_or_default();
            let source = match self.file_arg(&file) {
                Ok(source) => source,
                Err(reply) => return (reply, false),
            };
            let Ok(payload) = fs::read_to_string(&source) else {
                return (
                    format!(
                        "[{{\"type\":\"error\",\"error-type\":\"UnknownError\",\"message\":\"Файл не обнаружен '{}'\"}}]\n",
                        source.display()
                    ),
                    false,
                );
            };
            self.log(&format!("restored: {}", payload.trim()));
            // После загрузки агент закрывает сеанс и рвёт SSH-соединение сам (4.7.7.6).
            return (
                "[{\"type\":\"log\",\"message\":\"Требуется повторное подключение к агенту\"}]\n[{\"type\":\"success\",\"message\":\"\",\"body\":[]}]\n".to_owned(),
                true,
            );
        }
        if line.starts_with("config load-external-data-processor-or-report-from-files") {
            let xml = option("file").unwrap_or_default();
            let out = option("ext-file").unwrap_or_default();
            let (source, target) = match (self.file_arg(&xml), self.file_arg(&out)) {
                (Ok(source), Ok(target)) => (source, target),
                (Err(reply), _) | (_, Err(reply)) => return (reply, false),
            };
            let Ok(descriptor) = fs::read_to_string(&source) else {
                return (
                    "[{\"type\":\"error\",\"error-type\":\"ConfigFilesError\",\"message\":\"\"}]\n"
                        .to_owned(),
                    false,
                );
            };
            fs::write(
                &target,
                format!("EPF:{}", source.file_name().unwrap().to_string_lossy()),
            )
            .expect("ext file");
            self.external_sources
                .lock()
                .expect("external sources")
                .insert(target, descriptor);
            return (progress_then_success("Загрузка внешней обработки"), false);
        }
        if line.starts_with("config dump-external-data-processor-or-report-to-files") {
            let ext = option("ext-file").unwrap_or_default();
            let xml = option("file").unwrap_or_default();
            let (binary, target) = match (self.file_arg(&ext), self.file_arg(&xml)) {
                (Ok(binary), Ok(target)) => (binary, target),
                (Err(reply), _) | (_, Err(reply)) => return (reply, false),
            };
            let Some(descriptor) = self
                .external_sources
                .lock()
                .expect("external sources")
                .get(&binary)
                .cloned()
            else {
                return (
                    "[{\"type\":\"error\",\"error-type\":\"ConfigFilesError\",\"message\":\"\"}]\n"
                        .to_owned(),
                    false,
                );
            };
            fs::write(&target, descriptor).expect("descriptor xml");
            return (progress_then_success("Выгрузка внешней обработки"), false);
        }
        if line.starts_with("config extensions properties get") {
            if has("all-extensions") {
                return (self.extension_reply(None), false);
            }
            return (self.extension_reply(option("extension").as_deref()), false);
        }
        if line.starts_with("config extensions properties set") {
            let name = option("extension").unwrap_or_default();
            let mut extensions = self.extensions.lock().expect("extensions");
            let Some(extension) = extensions.iter_mut().find(|e| e.name == name) else {
                return (extension_not_found(), false);
            };
            let flag = |value: Option<String>| value.map(|v| v == "yes");
            if let Some(active) = flag(option("active")) {
                extension.active = active;
            }
            if let Some(safe_mode) = flag(option("safe-mode")) {
                extension.safe_mode = safe_mode;
            }
            if let Some(protection) = flag(option("unsafe-action-protection")) {
                extension.unsafe_action_protection = protection;
            }
            return (success(), false);
        }
        if line.starts_with("config extensions create") {
            // Как настоящий агент: синоним обязателен и только в форме NStr().
            if !option("synonym").is_some_and(|synonym| synonym.contains("='")) {
                return (
                    "[{\"type\":\"error\",\"error-type\":\"CommandFormatError\",\"message\":\"Неверный формат команды:Ошибка разбора параметра: synonym\"}]\n".to_owned(),
                    false,
                );
            }
            let name = option("extension").unwrap_or_default();
            self.extensions
                .lock()
                .expect("extensions")
                .push(FakeExtension {
                    name,
                    active: true,
                    safe_mode: true,
                    unsafe_action_protection: true,
                    purpose: option("purpose").unwrap_or_else(|| "customization".to_owned()),
                });
            return (success(), false);
        }
        if line.starts_with("config extensions delete") {
            let name = option("extension").unwrap_or_default();
            let mut extensions = self.extensions.lock().expect("extensions");
            let before = extensions.len();
            extensions.retain(|e| e.name != name);
            if extensions.len() == before {
                return (extension_not_found(), false);
            }
            return (success(), false);
        }
        if line == "common shutdown" {
            // Поддельный процесс платформы бывает только под unix; шлюз и чужой агент
            // своего процесса не имеют, гасить нечего.
            #[cfg(unix)]
            if let Ok(pid) = fs::read_to_string(&self.designer_pid_file) {
                let _ = std::process::Command::new("kill")
                    .arg(pid.trim())
                    .stderr(std::process::Stdio::null())
                    .status();
            }
            return (success(), true);
        }
        (
            "[{\"type\":\"error\",\"error-type\":\"CommandFormatError\",\"message\":\"Неизвестная команда\"}]\n".to_owned(),
            false,
        )
    }
}

fn extension_not_found() -> String {
    "[{\"type\":\"error\",\"error-type\":\"ExtensionNotFound\",\"message\":\"Операция не может быть выполнена, так как расширение конфигурации не найдено.\"}]\n".to_owned()
}

/// Слова команды с учётом двойных кавычек: `--synonym="ru='X'; en='X'"` — одно слово.
fn tokens(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for ch in line.chars() {
        match ch {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn success() -> String {
    "[\n{\n\"type\": \"success\",\n\"message\": \"\",\n\"body\": []\n}\n]\n".to_owned()
}

fn progress_then_success(what: &str) -> String {
    format!(
        "[{{\"type\":\"progress\",\"body\":{{\"message\":\"{what}\",\"percent\":0}}}}]\n[{{\"type\":\"progress\",\"body\":{{\"message\":\"{what}\",\"percent\":50}}}}]\n[{{\"type\":\"success\",\"message\":\"\",\"body\":[]}}]\n"
    )
}

impl server::Server for FakeAgent {
    type Handler = Self;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
        self.clone()
    }
}

impl server::Handler for FakeAgent {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        // Настоящий агент слушает порт только после старта процесса; двойник слушает
        // заранее, поэтому сессию он принимает лишь после того, как поддельный `1cv8`
        // записал свою раскладку.
        if let Some((expected_user, _)) = self.gate.as_ref() {
            // Шлюз с пользователями базы: только пользователь ИБ и его пароль.
            return Ok(if user == expected_user && password == AGENT_PASSWORD {
                Auth::Accept
            } else {
                Auth::reject()
            });
        }
        if self.base_dir.is_none() {
            let started = std::time::Instant::now();
            while !self.base_dir_file.exists() && started.elapsed() < Duration::from_secs(20) {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
        // Правило агента: база без пользователей принимает пустой логин и пустую или
        // настроенную пару; любое другое имя отвергается.
        if self.any_credentials
            || (self.accept_password && user.is_empty() && password == AGENT_PASSWORD)
        {
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.buffers
            .lock()
            .expect("buffers")
            .insert(channel.id(), Vec::new());
        self.channels
            .lock()
            .expect("channels")
            .insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    /// Подсистема SFTP того же соединения: файлы — в каталоге пользователя двойника,
    /// как у настоящей точки входа.
    async fn subsystem_request(
        &mut self,
        channel_id: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if name != "sftp" {
            session.channel_failure(channel_id)?;
            return Ok(());
        }
        let channel = self.channels.lock().expect("channels").remove(&channel_id);
        let Some(channel) = channel else {
            session.channel_failure(channel_id)?;
            return Ok(());
        };
        session.channel_success(channel_id)?;
        self.sftp_channels
            .lock()
            .expect("sftp channels")
            .push(channel_id);
        let handler = FakeSftp {
            root: self.user_dir(),
            read_only: self.sftp_read_only,
            commands_log: self.commands_log.clone(),
            handles: HashMap::new(),
            next_handle: 0,
            hold: self.sftp_hold.clone(),
        };
        tokio::spawn(async move {
            russh_sftp::server::run(channel.into_stream(), handler).await;
        });
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        // Баннер и приглашение до JSON-режима: раннер обязан их пропустить.
        session.data(
            channel,
            "1C:Enterprise 8.3 1C Designer Shell\ndesigner> "
                .as_bytes()
                .to_vec(),
        )?;
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self
            .sftp_channels
            .lock()
            .expect("sftp channels")
            .contains(&channel)
        {
            return Ok(());
        }
        let lines = {
            let mut buffers = self.buffers.lock().expect("buffers");
            let buffer = buffers.entry(channel).or_default();
            buffer.extend_from_slice(data);
            let mut lines = Vec::new();
            while let Some(end) = buffer.iter().position(|byte| *byte == b'\n') {
                let line = buffer.drain(..=end).collect::<Vec<_>>();
                lines.push(String::from_utf8_lossy(&line).trim_end().to_owned());
            }
            lines
        };
        for line in lines {
            let held = self
                .hold
                .clone()
                .filter(|hold| line.starts_with(&hold.command));
            if let Some(hold) = held {
                if let Err(error) = fs::write(&hold.started, "") {
                    eprintln!("fake agent: cannot mark the held command started: {error}");
                }
                wait_for_release(&hold.release).await;
                match hold.reply {
                    HoldReply::Normal => {}
                    HoldReply::Error => {
                        self.log(&line);
                        let failure = r#"[{"type":"error","error-type":"UnknownError","message":"held command failed"}]"#;
                        session.data(channel, format!("{failure}\n").into_bytes())?;
                        continue;
                    }
                    HoldReply::Close => {
                        self.log(&line);
                        session.eof(channel)?;
                        session.close(channel)?;
                        // Канал закрыт: остальные строки пакета читать уже некому.
                        break;
                    }
                    HoldReply::Prose { text, close } => {
                        // Работа сделана, как у настоящей команды; ответ о ней — проза.
                        let _ = self.respond(&line);
                        session.data(channel, text.as_bytes().to_vec())?;
                        if close {
                            session.eof(channel)?;
                            session.close(channel)?;
                            break;
                        }
                        continue;
                    }
                }
            }
            let (reply, closing) = self.respond(&line);
            session.data(channel, reply.into_bytes())?;
            if closing {
                session.eof(channel)?;
                session.close(channel)?;
            }
        }
        Ok(())
    }
}

/// SFTP-сервер двойника над каталогом пользователя. Пути клиента относительны корня;
/// журнал команд получает строки `sftp <op> <path>` — так тест видит, что и куда
/// переносилось.
struct FakeSftp {
    root: PathBuf,
    read_only: bool,
    commands_log: PathBuf,
    handles: HashMap<String, FakeSftpHandle>,
    next_handle: u64,
    hold: Option<SftpHold>,
}

enum FakeSftpHandle {
    File(std::fs::File),
    Dir {
        entries: Vec<(String, FileAttributes)>,
        sent: bool,
    },
}

impl FakeSftp {
    fn resolve(&self, path: &str) -> PathBuf {
        let trimmed = path.trim_start_matches('/');
        if trimmed.is_empty() || trimmed == "." {
            self.root.clone()
        } else {
            self.root.join(trimmed)
        }
    }

    fn log(&self, op: &str, path: &str) {
        if let Ok(mut log) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.commands_log)
        {
            use std::io::Write;
            let _ = writeln!(log, "sftp {op} {}", path.trim_start_matches('/'));
        }
    }

    fn new_handle(&mut self, handle: FakeSftpHandle) -> String {
        self.next_handle += 1;
        let id = format!("h{}", self.next_handle);
        self.handles.insert(id.clone(), handle);
        id
    }

    fn ok(id: u32) -> Status {
        Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "Ok".to_owned(),
            language_tag: "en-US".to_owned(),
        }
    }
}

impl russh_sftp::server::Handler for FakeSftp {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    async fn init(
        &mut self,
        _version: u32,
        _extensions: HashMap<String, String>,
    ) -> Result<Version, Self::Error> {
        Ok(Version::new())
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        pflags: OpenFlags,
        _attrs: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        let path = self.resolve(&filename);
        let writing = pflags.contains(OpenFlags::WRITE) || pflags.contains(OpenFlags::CREATE);
        if writing {
            if self.read_only {
                self.log("open-refused", &filename);
                return Err(StatusCode::Failure);
            }
            if let Some(hold) = self.hold.take() {
                if let Err(error) = fs::write(&hold.started, "") {
                    eprintln!("fake agent: cannot mark the held write started: {error}");
                }
                wait_for_release(&hold.release).await;
            }
            self.log("write", &filename);
            let file = fs::OpenOptions::new()
                .write(true)
                .create(pflags.contains(OpenFlags::CREATE))
                .truncate(pflags.contains(OpenFlags::TRUNCATE))
                .open(&path)
                .map_err(|_| StatusCode::Failure)?;
            let handle = self.new_handle(FakeSftpHandle::File(file));
            return Ok(Handle { id, handle });
        }
        self.log("read", &filename);
        let file = fs::File::open(&path).map_err(|_| StatusCode::NoSuchFile)?;
        let handle = self.new_handle(FakeSftpHandle::File(file));
        Ok(Handle { id, handle })
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, Self::Error> {
        self.handles.remove(&handle);
        Ok(Self::ok(id))
    }

    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, Self::Error> {
        use std::io::{Read, Seek, SeekFrom};
        let Some(FakeSftpHandle::File(file)) = self.handles.get_mut(&handle) else {
            return Err(StatusCode::Failure);
        };
        file.seek(SeekFrom::Start(offset))
            .map_err(|_| StatusCode::Failure)?;
        let mut data = vec![0; len as usize];
        let read = file.read(&mut data).map_err(|_| StatusCode::Failure)?;
        if read == 0 {
            return Err(StatusCode::Eof);
        }
        data.truncate(read);
        Ok(Data { id, data })
    }

    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<Status, Self::Error> {
        use std::io::{Seek, SeekFrom, Write};
        let Some(FakeSftpHandle::File(file)) = self.handles.get_mut(&handle) else {
            return Err(StatusCode::Failure);
        };
        file.seek(SeekFrom::Start(offset))
            .map_err(|_| StatusCode::Failure)?;
        file.write_all(&data).map_err(|_| StatusCode::Failure)?;
        Ok(Self::ok(id))
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let metadata =
            fs::symlink_metadata(self.resolve(&path)).map_err(|_| StatusCode::NoSuchFile)?;
        Ok(Attrs {
            id,
            attrs: FileAttributes::from(&metadata),
        })
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let metadata = fs::metadata(self.resolve(&path)).map_err(|_| StatusCode::NoSuchFile)?;
        Ok(Attrs {
            id,
            attrs: FileAttributes::from(&metadata),
        })
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, Self::Error> {
        let Some(FakeSftpHandle::File(file)) = self.handles.get(&handle) else {
            return Err(StatusCode::Failure);
        };
        let metadata = file.metadata().map_err(|_| StatusCode::Failure)?;
        Ok(Attrs {
            id,
            attrs: FileAttributes::from(&metadata),
        })
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, Self::Error> {
        let dir = self.resolve(&path);
        let mut entries = Vec::new();
        for entry in fs::read_dir(&dir).map_err(|_| StatusCode::NoSuchFile)? {
            let entry = entry.map_err(|_| StatusCode::Failure)?;
            let metadata = entry.metadata().map_err(|_| StatusCode::Failure)?;
            entries.push((
                entry.file_name().to_string_lossy().into_owned(),
                FileAttributes::from(&metadata),
            ));
        }
        let handle = self.new_handle(FakeSftpHandle::Dir {
            entries,
            sent: false,
        });
        Ok(Handle { id, handle })
    }

    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, Self::Error> {
        let Some(FakeSftpHandle::Dir { entries, sent }) = self.handles.get_mut(&handle) else {
            return Err(StatusCode::Failure);
        };
        if *sent {
            return Err(StatusCode::Eof);
        }
        *sent = true;
        Ok(Name {
            id,
            files: entries
                .iter()
                .map(|(name, attrs)| File::new(name.clone(), attrs.clone()))
                .collect(),
        })
    }

    async fn remove(&mut self, id: u32, filename: String) -> Result<Status, Self::Error> {
        self.log("remove", &filename);
        fs::remove_file(self.resolve(&filename)).map_err(|_| StatusCode::NoSuchFile)?;
        Ok(Self::ok(id))
    }

    async fn mkdir(
        &mut self,
        id: u32,
        path: String,
        _attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        self.log("mkdir", &path);
        fs::create_dir(self.resolve(&path)).map_err(|_| StatusCode::Failure)?;
        Ok(Self::ok(id))
    }

    async fn rmdir(&mut self, id: u32, path: String) -> Result<Status, Self::Error> {
        self.log("rmdir", &path);
        fs::remove_dir(self.resolve(&path)).map_err(|_| StatusCode::Failure)?;
        Ok(Self::ok(id))
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        let trimmed = path.trim_start_matches('/').trim_end_matches('/');
        let canonical = if trimmed.is_empty() || trimmed == "." {
            "/".to_owned()
        } else {
            format!("/{trimmed}")
        };
        Ok(Name {
            id,
            files: vec![File::dummy(canonical)],
        })
    }

    async fn rename(
        &mut self,
        id: u32,
        oldpath: String,
        newpath: String,
    ) -> Result<Status, Self::Error> {
        if self.read_only {
            return Err(StatusCode::Failure);
        }
        fs::rename(self.resolve(&oldpath), self.resolve(&newpath))
            .map_err(|_| StatusCode::Failure)?;
        Ok(Self::ok(id))
    }
}

/// Ключ хоста двойника: случайный на каждый запуск.
pub fn random_host_key() -> russh::keys::PrivateKey {
    let seed: [u8; 32] = rand::random();
    russh::keys::PrivateKey::new(
        russh::keys::ssh_key::private::KeypairData::Ed25519(
            russh::keys::ssh_key::private::Ed25519Keypair::from_seed(&seed),
        ),
        "fake-agent",
    )
    .expect("host key")
}

/// Отпечаток ключа в том виде, в каком его называют в конфигурации.
pub fn fingerprint_of(key: &russh::keys::PrivateKey) -> String {
    fingerprint_with(key, russh::keys::ssh_key::HashAlg::Sha256)
}

/// Кладёт закрытый ключ на диск в том виде, в каком его читает платформа.
pub fn write_host_key_file(path: &Path, key: &russh::keys::PrivateKey) {
    std::fs::write(
        path,
        key.to_openssh(russh::keys::ssh_key::LineEnding::LF)
            .expect("encode host key")
            .as_bytes(),
    )
    .expect("write host key");
}

/// Отпечаток выбранным алгоритмом: сверка обязана считать тем же, каким записано.
pub fn fingerprint_with(
    key: &russh::keys::PrivateKey,
    algorithm: russh::keys::ssh_key::HashAlg,
) -> String {
    key.public_key().fingerprint(algorithm).to_string()
}

/// Поднимает двойника на свободном порту в отдельном потоке; живёт до конца теста.
pub fn start_fake_agent(agent: FakeAgent) -> u16 {
    start_fake_agent_with_host_key(agent, random_host_key())
}

/// То же, но ключ хоста называет вызывающий: тесты закрепления сверяют именно его.
pub fn start_fake_agent_with_host_key(agent: FakeAgent, key: russh::keys::PrivateKey) -> u16 {
    start_fake_agent_on(agent, key, 0)
}

/// Файл, в который поддельный `1cv8` пишет порт и ключ хоста своего запуска: рядом с
/// файлом раскладки агента.
pub fn launch_request_file(base_dir_file: &Path) -> PathBuf {
    let mut name = base_dir_file.as_os_str().to_owned();
    name.push(".launch");
    PathBuf::from(name)
}

/// Двойник управляемого агента «поднимается» вместе с поддельным `1cv8`.
///
/// Раннер передаёт агенту порт (`/AgentPort`) и ключ хоста (`/AgentSSHHostKey`), а
/// поддельный `1cv8` пишет их в файл запроса. Поток двойника ждёт этот файл и поднимает
/// SSH-сервер на том порту с тем ключом — как настоящий агент. `presented` подменяет ключ:
/// так проверяется отказ чужому ключу. Поток живёт, пока жив каталог файла запроса.
pub fn serve_managed_launches(agent: FakeAgent, presented: Option<russh::keys::PrivateKey>) {
    let request = launch_request_file(&agent.base_dir_file);
    std::thread::spawn(move || {
        // Сервер прежнего запуска на том же порту останавливается: у нового запуска свой ключ.
        let mut running: HashMap<u16, tokio::sync::oneshot::Sender<()>> = HashMap::new();
        while request.parent().is_some_and(Path::exists) {
            if let Ok(text) = fs::read_to_string(&request) {
                let _ = fs::remove_file(&request);
                let mut lines = text.lines();
                let port: u16 = lines
                    .next()
                    .and_then(|line| line.trim().parse().ok())
                    .expect("fake 1cv8 names /AgentPort");
                let key = presented.clone().unwrap_or_else(|| {
                    match lines.next().map(str::trim).filter(|line| !line.is_empty()) {
                        Some(path) => russh::keys::load_secret_key(path, None)
                            .expect("host key handed to the agent"),
                        None => random_host_key(),
                    }
                });
                if let Some(stop) = running.remove(&port) {
                    let _ = stop.send(());
                }
                let (port, stop) = start_stoppable_fake_agent_on(agent.clone(), key, port);
                running.insert(port, stop);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    });
}

/// Двойник управляемого агента для команд, чьи настройки тест не пишет (`clone`): журналы и
/// раскладка — в своём каталоге, учётные данные — любые.
pub struct ManagedAgentDouble {
    pub commands_log: PathBuf,
    pub base_dir_file: PathBuf,
    pub pid_file: PathBuf,
    pub fail_dump: Arc<AtomicBool>,
    _dir: tempfile::TempDir,
}

pub fn managed_agent_double() -> ManagedAgentDouble {
    let dir = tempfile::tempdir().expect("agent dir");
    let root = dir.path().to_path_buf();
    let mut agent = FakeAgent::new(
        true,
        root.join("commands.log"),
        None,
        root.join("base-dir.txt"),
        root.join("designer.pid"),
    );
    agent.any_credentials = true;
    let fail_dump = Arc::clone(&agent.fail_dump);
    serve_managed_launches(agent, None);
    ManagedAgentDouble {
        commands_log: root.join("commands.log"),
        base_dir_file: root.join("base-dir.txt"),
        pid_file: root.join("designer.pid"),
        fail_dump,
        _dir: dir,
    }
}

/// Поднимает двойника на данном порту (`0` — на свободном) и возвращает порт.
fn start_fake_agent_on(agent: FakeAgent, key: russh::keys::PrivateKey, port: u16) -> u16 {
    let (port, stop) = start_stoppable_fake_agent_on(agent, key, port);
    // Двойник живёт до конца процесса теста: сброшенный отправитель остановил бы сервер.
    std::mem::forget(stop);
    port
}

/// То же, но сервер останавливается, когда сброшен (или сработал) возвращённый отправитель:
/// так управляемый двойник освобождает порт к следующему запуску на том же порту.
fn start_stoppable_fake_agent_on(
    agent: FakeAgent,
    key: russh::keys::PrivateKey,
    port: u16,
) -> (u16, tokio::sync::oneshot::Sender<()>) {
    let (port_tx, port_rx) = std::sync::mpsc::channel::<u16>();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("fake agent runtime");
        runtime.block_on(async move {
            // Порт прежнего запуска освобождается не мгновенно: привязка повторяется.
            let mut attempts = 0;
            let listener = loop {
                match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
                    Ok(listener) => break listener,
                    Err(error) if attempts < 200 => {
                        attempts += 1;
                        let _ = error;
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    Err(error) => panic!("bind fake agent: {error}"),
                }
            };
            port_tx
                .send(listener.local_addr().expect("addr").port())
                .expect("port");
            let config = Arc::new(server::Config {
                auth_rejection_time: Duration::from_millis(50),
                auth_rejection_time_initial: Some(Duration::ZERO),
                inactivity_timeout: Some(Duration::from_secs(120)),
                keys: vec![key],
                ..server::Config::default()
            });
            let mut agent = agent;
            tokio::select! {
                _ = agent.run_on_socket(config, &listener) => {}
                _ = stop_rx => {}
            }
        });
    });
    (port_rx.recv().expect("fake agent port"), stop_tx)
}

/// Поддельный `1cv8` в агентском режиме: записывает ключи, создаёт раскладку
/// `AgentBaseDir` как платформа и живёт до сигнала.
#[cfg(unix)]
pub fn write_fake_designer(path: &Path, args_log: &Path, pid_file: &Path, base_dir_file: &Path) {
    write_fake_designer_for_user(path, args_log, pid_file, base_dir_file, "");
}

/// То же, но карта `agentbasedir.json` называет пользователя базы `user`: так платформа
/// раскладывает каталог агента для базы с пользователями.
#[cfg(unix)]
pub fn write_fake_designer_for_user(
    path: &Path,
    args_log: &Path,
    pid_file: &Path,
    base_dir_file: &Path,
    user: &str,
) {
    let body = format!(
        r#"printf '%s\n' "$*" >> "{args_log}"
printf '%s\n' "$$" > "{pid_file}"
base=""
port=""
key=""
prev=""
for arg in "$@"; do
  if [ "$prev" = "/AgentBaseDir" ]; then base="$arg"; fi
  if [ "$prev" = "/AgentPort" ]; then port="$arg"; fi
  if [ "$prev" = "/AgentSSHHostKey" ]; then key="$arg"; fi
  prev="$arg"
done
if [ -z "$base" ]; then exit 3; fi
printf '%s\n%s\n' "$port" "$key" > "{launch}.tmp" && mv "{launch}.tmp" "{launch}"
mkdir -p "$base/0"
printf '{{"usersInfo":[{{"name":"{user}","dir":"0"}}]}}' > "$base/agentbasedir.json"
printf '%s' "$base" > "{base_dir_file}"
trap 'exit 0' TERM INT
while :; do sleep 1; done"#,
        args_log = args_log.display(),
        pid_file = pid_file.display(),
        base_dir_file = base_dir_file.display(),
        launch = launch_request_file(base_dir_file).display(),
    );
    super::write_shell_script(path, &body);
}

pub fn read_or_empty(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

#[cfg(unix)]
pub fn process_is_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

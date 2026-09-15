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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::server::{self, Auth, Msg, Server as _, Session};
use russh::{Channel, ChannelId};

pub const AGENT_PASSWORD: &str = "agentpass";

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
    /// Поколение конфигурации: растёт с каждой удачной загрузкой.
    pub generation: Arc<AtomicU64>,
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
            generation: Arc::new(AtomicU64::new(1)),
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
            .map(FakeExtension::record)
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

    fn user_dir(&self) -> PathBuf {
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
                format!("[{{\"type\":\"success\",\"body\":\"{}\"}}]\n", self.token()),
                false,
            );
        }
        if line.starts_with("config dump-config-to-files") {
            let dir = option("dir").unwrap_or_default();
            let target = self.user_dir().join(&dir);
            fs::create_dir_all(&target).expect("agent output dir");
            fs::write(target.join("Configuration.xml"), "<Configuration/>\n").expect("dump file");
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
            self.generation.fetch_add(1, Ordering::SeqCst);
            return (progress_then_success("Загрузка конфигурации"), false);
        }
        if line.starts_with("config update-db-cfg") {
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
        if self.base_dir.is_none() {
            let started = std::time::Instant::now();
            while !self.base_dir_file.exists() && started.elapsed() < Duration::from_secs(20) {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
        // Правило агента: база без пользователей принимает пустой логин и пустую или
        // настроенную пару; любое другое имя отвергается.
        if self.accept_password && user.is_empty() && password == AGENT_PASSWORD {
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
        reply.accept().await;
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

/// Поднимает двойника на свободном порту в отдельном потоке; живёт до конца теста.
pub fn start_fake_agent(agent: FakeAgent) -> u16 {
    let (port_tx, port_rx) = std::sync::mpsc::channel::<u16>();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("fake agent runtime");
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .expect("bind fake agent");
            port_tx
                .send(listener.local_addr().expect("addr").port())
                .expect("port");
            let seed: [u8; 32] = rand::random();
            let key = russh::keys::PrivateKey::new(
                russh::keys::ssh_key::private::KeypairData::Ed25519(
                    russh::keys::ssh_key::private::Ed25519Keypair::from_seed(&seed),
                ),
                "fake-agent",
            )
            .expect("host key");
            let config = Arc::new(server::Config {
                auth_rejection_time: Duration::from_millis(50),
                auth_rejection_time_initial: Some(Duration::ZERO),
                inactivity_timeout: Some(Duration::from_secs(120)),
                keys: vec![key],
                ..server::Config::default()
            });
            let mut agent = agent;
            let _ = agent.run_on_socket(config, &listener).await;
        });
    });
    port_rx.recv().expect("fake agent port")
}

/// Поддельный `1cv8` в агентском режиме: записывает ключи, создаёт раскладку
/// `AgentBaseDir` как платформа и живёт до сигнала.
pub fn write_fake_designer(path: &Path, args_log: &Path, pid_file: &Path, base_dir_file: &Path) {
    let body = format!(
        r#"printf '%s\n' "$*" >> "{args_log}"
printf '%s\n' "$$" > "{pid_file}"
base=""
prev=""
for arg in "$@"; do
  if [ "$prev" = "/AgentBaseDir" ]; then base="$arg"; fi
  prev="$arg"
done
if [ -z "$base" ]; then exit 3; fi
mkdir -p "$base/0"
printf '{{"usersInfo":[{{"name":"","dir":"0"}}]}}' > "$base/agentbasedir.json"
printf '%s' "$base" > "{base_dir_file}"
trap 'exit 0' TERM INT
while :; do sleep 1; done"#,
        args_log = args_log.display(),
        pid_file = pid_file.display(),
        base_dir_file = base_dir_file.display(),
    );
    super::write_shell_script(path, &body);
}

pub fn read_or_empty(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

pub fn process_is_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

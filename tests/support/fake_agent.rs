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
    buffers: Arc<Mutex<HashMap<ChannelId, Vec<u8>>>>,
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
            buffers: Arc::new(Mutex::new(HashMap::new())),
        }
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
        let option = |name: &str| -> Option<String> {
            line.split_whitespace()
                .find_map(|word| word.strip_prefix(&format!("--{name}=")))
                .map(str::to_owned)
        };
        let has = |flag: &str| {
            line.split_whitespace()
                .any(|word| word == format!("--{flag}"))
        };
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

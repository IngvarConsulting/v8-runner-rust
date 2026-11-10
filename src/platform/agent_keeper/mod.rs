//! Сторож живого агента Конфигуратора (#577): процесс раннера, который держит процесс агента и
//! его сессию SSH между командами (`INV.PLATFORM.A-MANAGED-AGENT-OUTLIVES-THE-COMMAND`).
//!
//! С агентом говорит только сторож. Команда обращается к нему через локальный канал, открытый
//! только своей учётной записи ОС ([`channel`]), построчным JSON ([`protocol`]). Сторож — скрытая
//! подкоманда `v8-runner agent keeper --key <hex>` ([`server`]); команда находит его по записи в
//! каталоге состояния учётной записи или поднимает ([`client`]).
//!
//! Раскладка под каталогом состояния `<state>`:
//!
//! - `<state>/agents/<key>/agent/` — каталог сторожа ([`ServicePaths::under`]): `record.json`
//!   (запись сторожа и его агента), `agent.log` (вывод сторожа), `base/` (`AgentBaseDir`),
//!   `workdir/` (текущий каталог агента), `agent.*.log` (журналы процесса агента);
//! - `<state>/agents/<key>/agent/host-keys/` — одноразовый ключ хоста агента;
//! - `<state>/agents/<key32>.sock` — сокет сторожа на Unix: путь сокета ограничен ~104 байтами,
//!   поэтому он короче каталога ключа. На Windows канал — `\\.\pipe\v8-runner-agent-<key>`;
//! - `<state>/bin/<версия>/<метка>/` — копия исполняемого файла раннера, из которой запущен
//!   сторож: так живой агент не мешает обновить раннер (`INV.PLATFORM.A-LIVE-AGENT-DIES-WITH-ITS-KEEPER`).
//!
//! Скрытая переменная окружения [`IDLE_ENV`] (`V8_RUNNER_AGENT_IDLE_MS`) задаёт сторожу срок
//! простоя в миллисекундах, не больше 15 минут. Она для тестов: переопределять срок в работе незачем.

mod channel;
pub mod client;
pub mod protocol;
mod server;

use std::path::{Path, PathBuf};
use std::time::Duration;

pub use server::run_keeper;

use crate::platform::live_registry::{agents_dir, agents_path};
use crate::platform::service_record::ServicePaths;

/// Срок простоя живого агента без держателя (`INV.PLATFORM.A-LIVE-AGENT-CLOSES-WHEN-NOBODY-HOLDS-IT`).
const IDLE_LIMIT: Duration = Duration::from_secs(15 * 60);

/// Скрытая переменная окружения: срок простоя сторожа в миллисекундах вместо [`IDLE_LIMIT`].
pub const IDLE_ENV: &str = "V8_RUNNER_AGENT_IDLE_MS";

/// Срок простоя этого сторожа: [`IDLE_ENV`] или 15 минут. Переменная срок только укорачивает:
/// продлить простой сверх 15 минут ею нельзя.
fn idle_limit() -> Duration {
    std::env::var(IDLE_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map_or(IDLE_LIMIT, |ms| Duration::from_millis(ms).min(IDLE_LIMIT))
}

/// Имя службы сторожа под каталогом ключа.
const SERVICE: &str = "agent";

/// Где живут сторож одного ключа и его агент.
#[derive(Debug, Clone)]
pub struct KeeperHome {
    state: PathBuf,
    key: String,
    paths: ServicePaths,
}

impl KeeperHome {
    /// Каталог сторожа ключа `key` под каталогом состояния `state`, созданный только для
    /// учётной записи. Ключ — шестнадцатеричная строка ([`account_key`]): иное имя каталога из
    /// ключа не строится.
    ///
    /// [`account_key`]: crate::support::machine::account::account_key
    pub fn open(state: &Path, key: &str) -> std::io::Result<Self> {
        let key = checked_key(key)?;
        let dir = agents_dir(state)?.join(&key);
        crate::support::machine::account::ensure_private_dir(&dir)?;
        let paths = ServicePaths::under(&dir, SERVICE);
        crate::support::machine::account::ensure_private_dir(paths.root())?;
        Ok(Self {
            state: state.to_path_buf(),
            key,
            paths,
        })
    }

    /// Каталог сторожа ключа `key`, только если он уже есть: ничего не создаёт. Его нет —
    /// сторожа этого ключа никогда не поднимали.
    pub fn existing(state: &Path, key: &str) -> std::io::Result<Option<Self>> {
        let key = checked_key(key)?;
        let paths = ServicePaths::under(&agents_path(state).join(&key), SERVICE);
        if !paths.root().is_dir() {
            return Ok(None);
        }
        Ok(Some(Self {
            state: state.to_path_buf(),
            key,
            paths,
        }))
    }

    pub fn state(&self) -> &Path {
        &self.state
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn paths(&self) -> &ServicePaths {
        &self.paths
    }

    /// Каталог ключа: в нём каталог сторожа и одноразовый ключ хоста.
    fn key_dir(&self) -> PathBuf {
        agents_path(&self.state).join(&self.key)
    }

    /// `AgentBaseDir` агента: каталог пользователя в нём называет карта платформы
    /// ([`user_dir`](crate::platform::agent::user_dir)).
    pub fn base_dir(&self) -> PathBuf {
        self.paths.root().join("base")
    }

    /// Канал сторожа: имя именованного канала Windows или путь сокета Unix.
    pub fn channel(&self) -> String {
        if cfg!(windows) {
            format!(r"\\.\pipe\v8-runner-agent-{}", self.key)
        } else {
            let short = &self.key[..self.key.len().min(32)];
            agents_path(&self.state)
                .join(format!("{short}.sock"))
                .display()
                .to_string()
        }
    }
}

/// Ключ сторожа — шестнадцатеричная строка: иное имя каталога из ключа не строится.
fn checked_key(key: &str) -> std::io::Result<String> {
    if key.len() < 16 || key.len() > 64 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("'{key}' is not a live agent key: 16 to 64 hexadecimal digits"),
        ));
    }
    Ok(key.to_ascii_lowercase())
}

/// Отпечаток учётных данных базы: HMAC с солью учётной записи, а не сами данные. С ним каждый
/// `run` приходит к сторожу, и тот сверяет его с отпечатком данных своей сессии
/// (`INV.PLATFORM.A-LIVE-AGENT-IS-TAKEN-ONLY-BY-ITS-KEY`).
pub fn credentials_digest(state: &Path, user: &str, password: &str) -> std::io::Result<String> {
    crate::support::machine::account::account_key(state, &["agent-credentials", user, password])
}

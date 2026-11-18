//! Сторож живого агента Конфигуратора (#577): процесс раннера, который держит процесс агента и
//! его сессию SSH между командами (`INV.PLATFORM.A-MANAGED-AGENT-OUTLIVES-THE-COMMAND`).
//!
//! С агентом говорит только сторож. Команда обращается к нему через локальный канал, открытый
//! только своей учётной записи ОС ([`channel`]), построчным JSON ([`protocol`]). Сторож — скрытая
//! подкоманда `v8-runner agent keeper --key <hex>` ([`server`]); команда находит его по записи в
//! рабочей копии или поднимает ([`client`]).
//!
//! Всё, что создаёт живой агент, лежит в рабочей копии (#735,
//! `INV.PLATFORM.RUNNER-FILES-STAY-IN-THE-WORKING-COPY`), под `workPath/agent/live`:
//!
//! - `<key16>/` — дом агента ([`KeeperHome::of`]): `record.json` (запись сторожа и его агента),
//!   `base/` (`AgentBaseDir`), `workdir/` (текущий каталог агента), `host-keys/` (одноразовый
//!   ключ хоста), `agent.*.log` (журналы процесса агента). Сторож убирает его, когда агент ушёл;
//! - `<key16>.log` — вывод сторожа, `<key16>.lock` — замок записи этого ключа;
//! - `bin/<версия>/<метка>/` — копия исполняемого файла раннера, из которой запущен сторож: так
//!   живой агент не мешает обновить раннер (`INV.PLATFORM.A-LIVE-AGENT-DIES-WITH-ITS-KEEPER`).
//!
//! Текущий каталог сторожа — `workPath/agent/live`: пока сторож жив, рабочую копию не удалить, и
//! уборка рабочей копии начинается с `v8-runner agent stop`. Общего файла учётной записи нет:
//! сторожей учётной записи находят по их каналам ([`Registry`]).
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

use crate::platform::live_registry::{
    is_key, legacy_agents_path, legacy_channel, lock_legacy_registry, Registry,
};
use crate::platform::service_record::ServicePaths;
use crate::support::fs::{acquire_advisory_lock, AdvisoryLockGuard};

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

/// Имя службы сторожа 0.17.0–0.17.3 под каталогом ключа.
const LEGACY_SERVICE: &str = "agent";

/// Сколько знаков ключа называет дом агента в рабочей копии: ключи агентов одной рабочей копии
/// различаются уже на них.
const SHORT_KEY: usize = 16;

/// Где живут сторож одного ключа и его агент.
#[derive(Debug, Clone)]
pub struct KeeperHome {
    registry: Registry,
    key: String,
    /// `workPath/agent/live`; у сторожа 0.17.0–0.17.3 — `<state>/agents/<key>`.
    live: PathBuf,
    paths: ServicePaths,
    layout: Layout,
}

/// Чья раскладка у дома.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layout {
    /// Дом в рабочей копии, `workPath/agent/live/<key16>`.
    WorkingCopy,
    /// Дом сторожа 0.17.0–0.17.3 в каталоге состояния: его находят и снимают, но запись его не
    /// пишут и дом не убирают.
    Former,
}

impl KeeperHome {
    /// Каталог живых агентов рабочей копии: `workPath/agent/live`.
    pub fn live_dir(work_path: &Path) -> PathBuf {
        work_path.join("agent").join("live")
    }

    /// Дом агента ключа `key` рабочей копии `work_path`: `workPath/agent/live/<key16>`.
    /// Единственный владелец этого пути: по нему сторож поднимает агента, а команда читает
    /// запись и `AgentBaseDir`. Ничего не создаёт: каталоги заводит [`Self::prepare`] под
    /// замком ключа ([`Self::lock`]). Ключ — шестнадцатеричная строка ([`account_key`]): иное
    /// имя каталога из ключа не строится.
    ///
    /// [`account_key`]: crate::support::machine::account::account_key
    pub fn of(registry: &Registry, work_path: &Path, key: &str) -> std::io::Result<Self> {
        let key = checked_key(key)?;
        Ok(Self::at(registry, key, Self::live_dir(work_path)))
    }

    /// Дом агента, который назвал сам сторож (`whereabouts`): `root` — его каталог
    /// `…/agent/live/<key16>`.
    pub fn found(registry: &Registry, key: &str, root: &Path) -> std::io::Result<Self> {
        let key = checked_key(key)?;
        let live = root.parent().map(Path::to_path_buf).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("the keeper names '{}' as its home", root.display()),
            )
        })?;
        let home = Self::at(registry, key, live);
        if home.paths.root() != root {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "the keeper names '{}' as its home, not a home of its key",
                    root.display()
                ),
            ));
        }
        Ok(home)
    }

    fn at(registry: &Registry, key: String, live: PathBuf) -> Self {
        let paths = ServicePaths::under(&live, short(&key));
        Self {
            registry: registry.clone(),
            key,
            live,
            paths,
            layout: Layout::WorkingCopy,
        }
    }

    /// Дом сторожа 0.17.0–0.17.3 ключа `key` в каталоге состояния, если он там есть.
    pub fn legacy(registry: &Registry, key: &str) -> Option<Self> {
        let key = checked_key(key).ok()?;
        let live = legacy_agents_path(registry.state()).join(&key);
        let paths = ServicePaths::under(&live, LEGACY_SERVICE);
        paths.root().is_dir().then(|| Self {
            registry: registry.clone(),
            key,
            live,
            paths,
            layout: Layout::Former,
        })
    }

    /// Заводит каталоги дома только для учётной записи. Зовут его под замком ключа: сторож,
    /// который уходит, убирает дом под тем же замком.
    pub fn prepare(&self) -> std::io::Result<()> {
        crate::support::machine::account::ensure_private_dir(&self.live)?;
        crate::support::machine::account::ensure_private_dir(self.paths.root())
    }

    /// Замок записи этого ключа: `workPath/agent/live/<key16>.lock`, вне дома, чтобы дом можно
    /// было убрать под ним. Берут его после замка учётной записи ([`Registry::lock`]), не до.
    /// Сторож 0.17.0–0.17.3 писал запись под общим замком каталога состояния.
    pub fn lock(&self) -> std::io::Result<AdvisoryLockGuard> {
        if self.layout == Layout::Former {
            return lock_legacy_registry(self.registry.state()).unwrap_or_else(|| {
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "the former live agent registry is gone",
                ))
            });
        }
        acquire_advisory_lock(&self.live.join(format!("{}.lock", short(&self.key))))
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn paths(&self) -> &ServicePaths {
        &self.paths
    }

    /// Каталог живых агентов рабочей копии: текущий каталог сторожа.
    pub fn live(&self) -> &Path {
        &self.live
    }

    /// Вывод сторожа: `workPath/agent/live/<key16>.log`, вне дома — он переживает уборку дома и
    /// остаётся уликой, когда агент не поднялся.
    pub fn log(&self) -> PathBuf {
        if self.layout == Layout::Former {
            return self.paths.log();
        }
        self.live.join(format!("{}.log", short(&self.key)))
    }

    /// `AgentBaseDir` агента: каталог пользователя в нём называет карта платформы
    /// ([`user_dir`](crate::platform::agent::user_dir)).
    pub fn base_dir(&self) -> PathBuf {
        self.paths.root().join("base")
    }

    /// Текущий каталог процесса агента.
    pub fn workdir(&self) -> PathBuf {
        self.paths.root().join("workdir")
    }

    /// Каталог одноразового ключа хоста агента.
    pub fn host_keys(&self) -> PathBuf {
        self.paths.root().join("host-keys")
    }

    /// Канал сторожа: имя именованного канала Windows или путь сокета Unix.
    pub fn channel(&self) -> String {
        if self.layout == Layout::Former {
            return legacy_channel(self.registry.state(), &self.key);
        }
        self.registry.channel(&self.key)
    }

    /// Убирает дом агента. Зовут под замком ключа, когда записи нет или она своя: сторож, который
    /// уходит, и тот, кто нашёл дом без живого сторожа. Файлы, которые ещё держит уходящий процесс
    /// агента, Windows отдаёт не сразу: удаление повторяется недолго, а остаток уберёт следующий
    /// сторож этого ключа.
    pub fn remove(&self) {
        if self.layout == Layout::Former {
            return;
        }
        let root = self.paths.root();
        for attempt in 0..20 {
            match std::fs::remove_dir_all(root) {
                Ok(()) => return,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
                Err(error) if attempt == 19 => {
                    tracing::debug!(home = %root.display(), %error, "the live agent home is not removed");
                }
                Err(_) => std::thread::sleep(Duration::from_millis(100)),
            }
        }
    }
}

/// Первые [`SHORT_KEY`] знаков ключа.
fn short(key: &str) -> &str {
    &key[..key.len().min(SHORT_KEY)]
}

/// Ключ сторожа — шестнадцатеричная строка: иное имя каталога из ключа не строится.
fn checked_key(key: &str) -> std::io::Result<String> {
    if !is_key(key) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("'{key}' is not a live agent key: 16 to 64 hexadecimal digits"),
        ));
    }
    Ok(key.to_ascii_lowercase())
}

/// Где лежит соль, из которой считаются ключ живого агента и отпечаток учётных данных:
/// `localPath/agent` (#735). Соль переживает `workPath`, и ключи рабочей копии от сборки к
/// сборке не меняются.
pub fn salt_dir(local_path: &Path) -> PathBuf {
    local_path.join("agent")
}

/// Отпечаток учётных данных базы: HMAC с солью рабочей копии, а не сами данные. Его считает
/// команда и отдаёт сторожу при подъёме; с ним каждый `run` приходит к сторожу, и тот сверяет его
/// с отпечатком данных своей сессии (`INV.PLATFORM.A-LIVE-AGENT-IS-TAKEN-ONLY-BY-ITS-KEY`).
pub fn credentials_digest(salt_dir: &Path, user: &str, password: &str) -> std::io::Result<String> {
    crate::support::machine::account::account_key(salt_dir, &["agent-credentials", user, password])
}

/// Слушатель канала для тестов соседних модулей.
#[cfg(all(test, any(unix, windows)))]
pub(crate) fn bind_for_test(name: &str) -> std::io::Result<impl Drop> {
    struct Bound(channel::Listener);
    impl Drop for Bound {
        fn drop(&mut self) {
            self.0.remove();
        }
    }
    channel::Listener::bind(name).map(Bound)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Дом живого агента — одна функция рабочей копии и ключа: команда и сторож находят один и
    /// тот же каталог в `workPath`, а не в каталоге учётной записи (#735).
    #[test]
    fn the_home_of_a_live_agent_is_a_function_of_the_working_copy_and_the_key() {
        let work = Path::new("/work/copy");
        let key = "0123456789abcdef".repeat(4);
        let registry = Registry::at(PathBuf::from("/state"));
        let home = KeeperHome::of(&registry, work, &key).unwrap();
        let root = work.join("agent").join("live").join("0123456789abcdef");
        assert_eq!(home.paths().root(), root);
        assert_eq!(home.base_dir(), root.join("base"));
        assert_eq!(home.workdir(), root.join("workdir"));
        assert_eq!(home.host_keys(), root.join("host-keys"));
        assert_eq!(home.paths().record(), root.join("record.json"));
        assert_eq!(
            home.log(),
            work.join("agent").join("live").join("0123456789abcdef.log")
        );
        assert!(!home.base_dir().starts_with("/state"));
        let found = KeeperHome::found(&registry, &key, &root).unwrap();
        assert_eq!(found.paths().root(), home.paths().root());
        assert!(
            KeeperHome::found(&registry, &key, &work.join("elsewhere")).is_err(),
            "a home that is not of its key is refused"
        );
        assert!(KeeperHome::of(&registry, work, "../../x").is_err());
    }
}

//! `infobase create --recreate`: прежняя база этой копии откладывается, создаётся новая, и
//! отложенное удаляется после удачи или возвращается после неудачи.
//!
//! Порядок (`INV.USE-CASES.RECREATE-REFUSES-A-BUSY-BASE-BEFORE-REMOVING`): замки и проверка,
//! чья база, берёт граница команды; здесь — вид цели, остаток прошлого пересоздания, сеансы
//! базы, затем снятие простаивающих агентов, затем точка невозврата — память о базе стирается,
//! база откладывается. У файловой базы откладываются только записи `1Cv8*` верхнего уровня
//! (`INV.USE-CASES.RECREATE-REMOVES-ONLY-THE-PLATFORM-FILES-OF-THE-BASE`), у управляемого
//! стенда — своя БД и регистрация (`INV.USE-CASES.A-MANAGED-RECREATE-DROPS-ONLY-ITS-OWN-DATABASE`).
//! Неудача создания возвращает отложенное (`INV.USE-CASES.RECREATE-ROLLS-BACK-A-FAILED-CREATION`).

use std::path::{Path, PathBuf};

use crate::config::model::AppConfig;
use crate::domain::capability::TargetKind;
use crate::domain::init::{RemovedEntry, RemovedKind, RemovedState};
use crate::platform::file_base_hold::{
    is_sharing_violation, platform_entries, sessions_hold, Hold,
};
use crate::use_cases::context::ExecutionContext;
use crate::use_cases::exchange_guard::forget_base_memory;
use crate::use_cases::infobase_lock::{idle_live_agents, settle_live_agents};
use crate::use_cases::infobase_owner::holders;
use crate::use_cases::managed_stand::{old_database, OldDatabase};
use crate::use_cases::result::{UseCaseError, UseCaseErrorKind};

/// Прежняя база, проверенная и готовая к откладыванию.
pub(super) struct Recreation {
    old: OldBase,
    /// Простаивающие живые агенты этой базы, которых снимет пересоздание.
    agents: Vec<Option<u32>>,
    /// Чья база по метке, для превью: `None` — своя.
    owner: Option<String>,
}

enum OldBase {
    File(FileBase),
    Managed(OldDatabase),
}

struct FileBase {
    dir: PathBuf,
    aside: PathBuf,
    entries: Vec<PathBuf>,
}

/// Проверки `--recreate` до любых изменений: вид цели, остаток прошлого пересоздания,
/// сеансы. Отказ здесь ничего не трогает.
pub(super) fn prepare(
    context: &ExecutionContext,
    config: &AppConfig,
    dry_run: bool,
) -> Result<Recreation, UseCaseError> {
    let old = match config.target_kind() {
        TargetKind::File => OldBase::File(file_base(config)?),
        TargetKind::Cluster | TargetKind::Standalone if managed_target(config) => {
            OldBase::Managed(old_database(context, config)?)
        }
        TargetKind::Cluster | TargetKind::Standalone => {
            return Err(UseCaseError::from(crate::support::error::AppError::capability_for(
                crate::support::error::CapabilityReason::Target,
                "infobase create --recreate replaces only an infobase the runner owns: a file infobase, or a runner-managed cluster or standalone server (infobase.cluster.managed, infobase.standalone.managed) over the runner's PostgreSQL; a server infobase you run is removed with your own tools",
            )));
        }
    };
    Ok(Recreation {
        old,
        agents: idle_live_agents(config),
        owner: dry_run.then(|| other_owner(config)).flatten(),
    })
}

/// Другая копия, которая держит файловую базу по её метке; `None` — база своя или без метки
/// (такая считается своей). Метку только читает: проверку и предупреждение даёт граница.
fn other_owner(config: &AppConfig) -> Option<String> {
    let holders = holders(config)?;
    let Some(owners) = holders.owners else {
        return Some(format!(
            "the owner marker '{}' is not readable",
            holders.marker.display()
        ));
    };
    let others: Vec<String> = owners
        .iter()
        .filter(|owner| !owner.this_copy)
        .map(|owner| match &owner.host {
            Some(host) => format!("'{}' on {host}", owner.project.display()),
            None => format!("'{}'", owner.project.display()),
        })
        .collect();
    (!others.is_empty()).then(|| format!("the working copy {}", others.join(", ")))
}

/// Стенд раннера: кластер или автономный сервер с `managed`.
fn managed_target(config: &AppConfig) -> bool {
    let infobase = &config.infobase;
    infobase
        .cluster
        .as_ref()
        .is_some_and(|cluster| cluster.managed.is_some())
        || infobase
            .standalone
            .as_ref()
            .is_some_and(|standalone| standalone.managed.is_some())
}

fn file_base(config: &AppConfig) -> Result<FileBase, UseCaseError> {
    let dir = config
        .v8_connection()
        .file_path()
        .map(PathBuf::from)
        .ok_or_else(|| runtime("a file target names no infobase path".to_owned()))?;
    let aside = aside_dir(&dir).ok_or_else(|| {
        runtime(format!(
            "the infobase directory '{}' has no parent to set the old infobase aside in",
            dir.display()
        ))
    })?;
    if aside.exists() {
        return Err(UseCaseError::new(
            UseCaseErrorKind::Validation,
            format!(
                "'{}' is left by an earlier infobase create --recreate that did not finish: it holds the files of the old infobase of '{}'; move them back into the infobase directory or delete the directory, then repeat",
                aside.display(),
                dir.display()
            ),
        ));
    }
    match sessions_hold(&dir) {
        Ok(Hold::Absent | Hold::Free) => {}
        Ok(Hold::Held { pid }) => return Err(file_sessions_refusal(config, &dir, pid)),
        Err(error) => {
            return Err(runtime(format!(
                "cannot probe the file infobase '{}': {error}; infobase create --recreate removes nothing it cannot check",
                dir.display()
            )))
        }
    }
    let entries = platform_entries(&dir).map_err(|error| {
        runtime(format!(
            "cannot list the infobase directory '{}': {error}",
            dir.display()
        ))
    })?;
    Ok(FileBase {
        dir,
        aside,
        entries,
    })
}

/// Каталог отложенной базы рядом с её каталогом, на том же томе:
/// `.<имя каталога>.v8-runner.recreate`.
fn aside_dir(dir: &Path) -> Option<PathBuf> {
    let parent = dir.parent()?;
    let mut name = std::ffi::OsString::from(".");
    name.push(dir.file_name()?);
    name.push(".v8-runner.recreate");
    Some(parent.join(name))
}

/// Остаток прошлого пересоздания рядом с файловой базой: его называет и простой `create`.
pub(super) fn leftover_of(dir: &Path) -> Option<PathBuf> {
    aside_dir(dir).filter(|aside| aside.exists())
}

/// Отказ `infobase_sessions` файловой базы. Форму `error.sessions` строит владелец из
/// применения: перечня у файловой базы нет (`list: null`), завершать сеансы раннер не берётся,
/// блокировка начала сеансов — по файлу `1Cv8.cdn`.
fn file_sessions_refusal(config: &AppConfig, dir: &Path, pid: Option<i32>) -> UseCaseError {
    let holder = pid.map_or_else(String::new, |pid| format!(" (process {pid})"));
    UseCaseError::new(
        UseCaseErrorKind::InfobaseSessions,
        format!(
            "infobase create --recreate removes nothing: the file infobase '{}' is open{holder}; close its sessions (clients, Designer, a standalone server) and repeat",
            dir.display()
        ),
    )
    .with_sessions(crate::use_cases::apply::sessions::infobase_sessions(
        config,
        None,
        false,
        crate::domain::sessions::SessionsEvidence::Executor,
        None,
    ))
}

fn runtime(message: String) -> UseCaseError {
    UseCaseError::new(UseCaseErrorKind::Runtime, message)
}

impl Recreation {
    /// Превью: что было бы отложено и удалено, каких агентов раннер снял бы и чья база.
    pub(super) fn plan(&self) -> (String, Vec<RemovedEntry>) {
        let entries = self.parts(RemovedState::Planned);
        let owner = match &self.owner {
            None => "the infobase is this working copy's own".to_owned(),
            Some(owner) => format!(
                "the infobase is held by {owner}: it would be replaced with the warning named in warnings"
            ),
        };
        let what = if entries.is_empty() {
            "there is no infobase to remove".to_owned()
        } else {
            format!(
                "would set aside and, once the new infobase is created, remove: {}",
                entries
                    .iter()
                    .map(|entry| entry.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        (format!("{owner}; {what}"), entries)
    }

    fn parts(&self, state: RemovedState) -> Vec<RemovedEntry> {
        let mut parts: Vec<RemovedEntry> = match &self.old {
            OldBase::File(base) => base
                .entries
                .iter()
                .map(|path| entry(RemovedKind::File, path.display().to_string(), state))
                .collect(),
            OldBase::Managed(old) => old
                .parts()
                .into_iter()
                .map(|(kind, name)| entry(kind, name, state))
                .collect(),
        };
        parts.extend(
            self.agents
                .iter()
                .map(|pid| entry(RemovedKind::LiveAgent, agent_name(*pid), state)),
        );
        parts
    }

    /// Снятие простаивающих агентов, затем точка невозврата: память о базе стирается, база
    /// откладывается. Неудача до точки невозврата ничего не меняет в базе.
    pub(super) fn set_aside(
        self,
        context: &ExecutionContext,
        config: &AppConfig,
    ) -> Result<SetAside, (UseCaseError, Vec<RemovedEntry>)> {
        let agents = settle_live_agents(
            config,
            "infobase create",
            crate::use_cases::infobase_lock::LiveAgents::Remove,
        )
        .map_err(|error| (UseCaseError::from(error), Vec::new()))?;
        let agents: Vec<RemovedEntry> = agents
            .into_iter()
            .map(|pid| {
                entry(
                    RemovedKind::LiveAgent,
                    agent_name(pid),
                    RemovedState::Removed,
                )
            })
            .collect();
        let set_aside = match &self.old {
            OldBase::File(base) => base.set_aside(config),
            OldBase::Managed(old) => old.set_aside(context, config),
        };
        if let Err(error) = set_aside {
            return Err((error, agents));
        }
        // Память стирается, только когда база уже отложена: отказ откладывания её не трогает.
        let mut warnings: Vec<String> = forget_base_memory(config).into_iter().collect();
        warnings.extend((self.agents.len() > agents.len()).then(|| {
            "a live agent of this infobase was not removed; the old one answers to nothing now"
                .to_owned()
        }));
        Ok(SetAside {
            old: self.old,
            agents,
            warnings,
        })
    }
}

fn entry(kind: RemovedKind, name: String, state: RemovedState) -> RemovedEntry {
    RemovedEntry { kind, name, state }
}

fn agent_name(pid: Option<u32>) -> String {
    pid.map_or_else(
        || "a live Designer agent".to_owned(),
        |pid| format!("the live Designer agent pid {pid}"),
    )
}

impl FileBase {
    /// Записи `1Cv8*` переносятся в каталог рядом. Запись, которую держит процесс, — отказ
    /// `infobase_sessions`; перенесённое до неё возвращается.
    fn set_aside(&self, config: &AppConfig) -> Result<(), UseCaseError> {
        if self.entries.is_empty() {
            return Ok(());
        }
        // Сеанс мог войти после проверки; на POSIX перенос открытого файла не отказывает,
        // поэтому база спрашивается ещё раз прямо перед ним.
        if let Ok(Hold::Held { pid }) = sessions_hold(&self.dir) {
            return Err(file_sessions_refusal(config, &self.dir, pid));
        }
        std::fs::create_dir(&self.aside).map_err(|error| {
            runtime(format!(
                "cannot create '{}' to set the old infobase aside: {error}",
                self.aside.display()
            ))
        })?;
        let mut moved = Vec::new();
        for path in &self.entries {
            let Some(name) = path.file_name() else {
                continue;
            };
            if let Err(error) = std::fs::rename(path, self.aside.join(name)) {
                let back = self.move_back(&moved);
                if back.is_empty() {
                    let _ = std::fs::remove_dir(&self.aside);
                }
                let refusal = if is_sharing_violation(&error) {
                    file_sessions_refusal(config, &self.dir, None)
                } else {
                    runtime(format!(
                        "cannot set '{}' aside: {error}; nothing is removed",
                        path.display()
                    ))
                };
                return Err(if back.is_empty() {
                    refusal
                } else {
                    refusal.followed_by(format!(
                        "and these files stay in '{}': {}",
                        self.aside.display(),
                        back.join(", ")
                    ))
                });
            }
            moved.push(name.to_owned());
        }
        Ok(())
    }

    /// Возвращает перенесённые записи; ответ — те, что вернуть не вышло.
    fn move_back(&self, moved: &[std::ffi::OsString]) -> Vec<String> {
        moved
            .iter()
            .filter(|name| std::fs::rename(self.aside.join(name), self.dir.join(name)).is_err())
            .map(|name| name.to_string_lossy().into_owned())
            .collect()
    }
}

/// Отложенная прежняя база на время создания новой.
pub(super) struct SetAside {
    old: OldBase,
    agents: Vec<RemovedEntry>,
    /// Что не удалось при откладывании, но базе не мешает: недотёртая память.
    pub(super) warnings: Vec<String>,
}

/// Итог пересоздания после создания: что удалено или возвращено и что сказать сверх того.
pub(super) struct Finished {
    pub(super) removed: Vec<RemovedEntry>,
    pub(super) warnings: Vec<String>,
}

impl SetAside {
    /// Удача: отложенное удаляется. Неудача: то, что оставило создание, убирается, отложенное
    /// возвращается, а память о вернувшейся базе стирается — она не та, что помнил каталог.
    pub(super) fn finish(
        self,
        context: &ExecutionContext,
        config: &AppConfig,
        created: bool,
    ) -> Finished {
        let mut warnings = self.warnings;
        let mut removed = self.agents;
        match (&self.old, created) {
            (OldBase::File(base), true) => {
                let state = match std::fs::remove_dir_all(&base.aside) {
                    Err(error) if base.aside.exists() => {
                        warnings.push(format!(
                            "the files of the old infobase were not deleted and stay in '{}': {error}; delete the directory",
                            base.aside.display()
                        ));
                        RemovedState::Left
                    }
                    _ => RemovedState::Removed,
                };
                removed.extend(
                    base.entries
                        .iter()
                        .map(|path| entry(RemovedKind::File, path.display().to_string(), state)),
                );
            }
            (OldBase::File(base), false) => {
                let left = base.restore();
                if !left.is_empty() {
                    warnings.push(format!(
                        "the old infobase was not fully put back: {}",
                        left.join("; ")
                    ));
                }
                removed.extend(base.entries.iter().map(|path| {
                    let state = if path.exists() {
                        RemovedState::Restored
                    } else {
                        RemovedState::Left
                    };
                    entry(RemovedKind::File, path.display().to_string(), state)
                }));
                warnings.extend(forget_base_memory(config));
            }
            (OldBase::Managed(old), true) => {
                let left = old.discard(context, config);
                let state = if left.is_some() {
                    RemovedState::Left
                } else {
                    RemovedState::Removed
                };
                warnings.extend(left);
                removed.extend(
                    old.parts()
                        .into_iter()
                        .map(|(kind, name)| entry(kind, name, state)),
                );
            }
            (OldBase::Managed(old), false) => {
                let left = old.restore(context, config);
                let state = if left.is_empty() {
                    RemovedState::Restored
                } else {
                    RemovedState::Left
                };
                if !left.is_empty() {
                    warnings.push(format!(
                        "the old infobase was not fully put back: {}",
                        left.join("; ")
                    ));
                }
                removed.extend(
                    old.parts()
                        .into_iter()
                        .map(|(kind, name)| entry(kind, name, state)),
                );
                warnings.extend(forget_base_memory(config));
            }
        }
        Finished { removed, warnings }
    }
}

impl FileBase {
    /// Неудача создания: записи `1Cv8*`, которые оно оставило, убираются, отложенные
    /// возвращаются. Ответ — что осталось не на месте.
    fn restore(&self) -> Vec<String> {
        if self.entries.is_empty() {
            return Vec::new();
        }
        let mut left = Vec::new();
        match platform_entries(&self.dir) {
            Ok(created) => {
                for path in created {
                    let removed = if path.is_dir() {
                        std::fs::remove_dir_all(&path)
                    } else {
                        std::fs::remove_file(&path)
                    };
                    if let Err(error) = removed {
                        left.push(format!(
                            "'{}' left by the failed creation: {error}",
                            path.display()
                        ));
                    }
                }
            }
            Err(error) => left.push(format!("cannot list '{}': {error}", self.dir.display())),
        }
        let names: Vec<std::ffi::OsString> = self
            .entries
            .iter()
            .filter_map(|path| path.file_name().map(ToOwned::to_owned))
            .collect();
        let back = self.move_back(&names);
        if back.is_empty() {
            let _ = std::fs::remove_dir(&self.aside);
        } else {
            left.push(format!(
                "these files of the old infobase stay in '{}': {}; move them back into '{}'",
                self.aside.display(),
                back.join(", "),
                self.dir.display()
            ));
        }
        left
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_old_base_is_set_aside_next_to_its_directory() {
        assert_eq!(
            aside_dir(Path::new("/work/build/ib")),
            Some(PathBuf::from("/work/build/.ib.v8-runner.recreate"))
        );
    }
}

//! Допустимость глобальных ключей — по листу дерева `clap`.
//!
//! Глобальный ключ виден всякой команде, но не всякая умеет его исполнить. Принять ключ и
//! ничем его не исполнить значит соврать вызывающему, поэтому лист, у которого превью нет,
//! ключ отвергает и называет причину.
//!
//! Строка таблицы ключуется путём листа, а не вариантом перечисления `Command`: скрытые
//! синонимы схлопываются в один вариант (`init` и `config init`, `download` и
//! `infobase configuration export`), а `infobase create` приходит вариантом, которого в
//! дереве нет вовсе. Путь берётся из `ArgMatches` до нормализации имён — там синонимы уже
//! сведены `clap` к каноническому имени.

use clap::ArgMatches;

use crate::config::model::InfobaseSelector;
use crate::use_cases::result::{UseCaseError, UseCaseErrorKind};

/// Путь листа, как его разобрал `clap`: `check designer-config`, `mcp serve stdio`.
pub fn leaf_command_path(matches: &ArgMatches) -> String {
    let mut parts: Vec<&str> = Vec::new();
    let mut current = matches;
    while let Some((name, sub)) = current.subcommand() {
        parts.push(name);
        current = sub;
    }
    parts.join(" ")
}

/// Умеет ли лист превью.
#[derive(Clone, Copy)]
enum Preview {
    /// `--dry-run` показывает план и ничего не запускает.
    Runs,
    /// Превью нет; причина попадает в текст отказа.
    Absent(&'static str),
}

/// Что лист делает с ключом базы.
#[derive(Clone, Copy)]
enum Base {
    /// Селектор доходит до загрузчика конфигурации: базу можно назвать именем или строкой.
    /// Это верно и там, где сама команда базы не касается, — `select_infobase` разрешает
    /// базу при всякой загрузке конфига, и отказ от ключа оставил бы без выхода проект,
    /// где `origin` не объявлен.
    Resolves,
    /// Объявляет базу: имя разрешать не по чему, принимается только строка соединения.
    Declares,
    /// Базы не касается.
    Ignores,
}

struct Leaf {
    path: &'static str,
    preview: Preview,
    base: Base,
}

const LEAVES: &[Leaf] = &[
    Leaf {
        path: "version",
        preview: Preview::Absent("it prints the version and starts nothing"),
        base: Base::Ignores,
    },
    // `clone` объявляет базу своим ключом `--connection`, и он обязателен: назвать адрес
    // дважды нечем, поэтому глобальный ключ здесь отвергается, а не объявляет, как у `init`.
    Leaf {
        path: "clone",
        preview: Preview::Runs,
        base: Base::Ignores,
    },
    Leaf {
        path: "init",
        preview: Preview::Absent("it writes the project file and starts nothing"),
        base: Base::Declares,
    },
    Leaf {
        path: "config init",
        preview: Preview::Absent("it writes the project file and starts nothing"),
        base: Base::Declares,
    },
    Leaf {
        path: "tools download yaxunit",
        preview: Preview::Absent("its preview is not written yet"),
        base: Base::Resolves,
    },
    Leaf {
        path: "tools download vanessa",
        preview: Preview::Absent("its preview is not written yet"),
        base: Base::Resolves,
    },
    Leaf {
        path: "tools download client-mcp",
        preview: Preview::Absent("its preview is not written yet"),
        base: Base::Resolves,
    },
    Leaf {
        path: "extensions",
        preview: Preview::Runs,
        base: Base::Resolves,
    },
    Leaf {
        path: "extensions list",
        preview: Preview::Runs,
        base: Base::Resolves,
    },
    Leaf {
        path: "extensions info",
        preview: Preview::Runs,
        base: Base::Resolves,
    },
    Leaf {
        path: "extensions create",
        preview: Preview::Runs,
        base: Base::Resolves,
    },
    Leaf {
        path: "extensions delete",
        preview: Preview::Runs,
        base: Base::Resolves,
    },
    Leaf {
        path: "extensions activate",
        preview: Preview::Runs,
        base: Base::Resolves,
    },
    Leaf {
        path: "push",
        preview: Preview::Runs,
        base: Base::Resolves,
    },
    Leaf {
        path: "upload",
        preview: Preview::Runs,
        base: Base::Resolves,
    },
    Leaf {
        path: "pull",
        preview: Preview::Runs,
        base: Base::Resolves,
    },
    Leaf {
        path: "download",
        preview: Preview::Runs,
        base: Base::Resolves,
    },
    Leaf {
        path: "test yaxunit all",
        preview: Preview::Absent(
            "the run prepares its own directory and files, and its preview is not written yet",
        ),
        base: Base::Resolves,
    },
    Leaf {
        path: "test yaxunit module",
        preview: Preview::Absent(
            "the run prepares its own directory and files, and its preview is not written yet",
        ),
        base: Base::Resolves,
    },
    Leaf {
        path: "test va",
        preview: Preview::Absent(
            "the run prepares its own directory and files, and its preview is not written yet",
        ),
        base: Base::Resolves,
    },
    Leaf {
        path: "infobase create",
        preview: Preview::Runs,
        base: Base::Resolves,
    },
    Leaf {
        path: "infobase configuration export",
        preview: Preview::Runs,
        base: Base::Resolves,
    },
    Leaf {
        path: "infobase dump",
        preview: Preview::Runs,
        base: Base::Resolves,
    },
    Leaf {
        path: "infobase restore",
        preview: Preview::Runs,
        base: Base::Resolves,
    },
    Leaf {
        path: "convert",
        preview: Preview::Runs,
        base: Base::Resolves,
    },
    Leaf {
        path: "make",
        preview: Preview::Runs,
        base: Base::Resolves,
    },
    Leaf {
        path: "check",
        preview: Preview::Runs,
        base: Base::Resolves,
    },
    Leaf {
        path: "check designer-config",
        preview: Preview::Runs,
        base: Base::Resolves,
    },
    Leaf {
        path: "check designer-modules",
        preview: Preview::Runs,
        base: Base::Resolves,
    },
    Leaf {
        path: "check edt",
        preview: Preview::Runs,
        base: Base::Resolves,
    },
    Leaf {
        path: "launch",
        preview: Preview::Runs,
        base: Base::Resolves,
    },
    Leaf {
        path: "publish",
        preview: Preview::Runs,
        base: Base::Resolves,
    },
    Leaf {
        path: "mcp serve stdio",
        preview: Preview::Absent("the server starts nothing by itself"),
        base: Base::Resolves,
    },
    Leaf {
        path: "mcp serve http",
        preview: Preview::Absent("the server starts nothing by itself"),
        base: Base::Resolves,
    },
];

fn leaf(path: &str) -> Option<&'static Leaf> {
    LEAVES.iter().find(|leaf| leaf.path == path)
}

/// Отказ по глобальному ключу, который лист исполнить не может. Значение типизировано:
/// рисует его тот, кто и так рисует отказы этого пути.
///
/// Лист, которого в таблице нет, ключами не пользуется: молчаливое согласие — та самая
/// ложь, против которой написано правило, поэтому отказ здесь закрыт по умолчанию.
pub fn refusal(path: &str, dry_run: bool, infobase: Option<&str>) -> Option<UseCaseError> {
    let refuse = |message: String| Some(UseCaseError::new(UseCaseErrorKind::Validation, message));
    // Ключ без значения — не отсутствие ключа: вызывающий что-то назвал, и раннер обязан
    // сказать, что названного не понял.
    if infobase.is_some_and(|value| value.trim().is_empty()) {
        return refuse(format!("--infobase names no infobase for `{path}`"));
    }
    let Some(leaf) = leaf(path) else {
        if dry_run || infobase.is_some() {
            return refuse(format!(
                "`{path}` declares nothing about the global keys, so --dry-run and --infobase are not accepted there"
            ));
        }
        return None;
    };
    if dry_run {
        if let Preview::Absent(reason) = leaf.preview {
            return refuse(format!(
                "`{path}` has no preview: {reason}. Remove --dry-run"
            ));
        }
    }
    match (leaf.base, InfobaseSelector::from_flag(infobase)) {
        (Base::Ignores, InfobaseSelector::Name(_) | InfobaseSelector::Connection(_)) => {
            refuse(format!("`{path}` selects no infobase. Remove --infobase"))
        }
        (Base::Declares, InfobaseSelector::Name(name)) => refuse(format!(
            "`{path}` declares the infobase, so --infobase takes a connection string here, not the name `{name}`"
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::args::Cli;
    use clap::CommandFactory;

    fn leaves(command: &clap::Command, prefix: &str, found: &mut Vec<String>) {
        let subcommands: Vec<&clap::Command> = command
            .get_subcommands()
            // `help` дописывает сам `clap` каждому узлу с подкомандами.
            .filter(|sub| sub.get_name() != "help")
            .collect();
        // Узел несёт строку, если ниже него команд нет или подкоманда необязательна:
        // `extensions` без подкоманды — законная команда.
        if !prefix.is_empty() && (subcommands.is_empty() || !command.is_subcommand_required_set()) {
            found.push(prefix.to_owned());
        }
        for sub in subcommands {
            let path = if prefix.is_empty() {
                sub.get_name().to_owned()
            } else {
                format!("{prefix} {}", sub.get_name())
            };
            leaves(sub, &path, found);
        }
    }

    fn declared_leaves() -> Vec<String> {
        let mut command = Cli::command();
        command.build();
        let mut found = Vec::new();
        leaves(&command, "", &mut found);
        found
    }

    #[test]
    fn every_leaf_of_the_command_tree_declares_what_it_does_with_the_global_keys() {
        let declared = declared_leaves();
        let missing: Vec<&String> = declared
            .iter()
            .filter(|path| leaf(path).is_none())
            .collect();
        assert!(
            missing.is_empty(),
            "листья без строки в таблице: {missing:?}"
        );
        let mut paths: Vec<&str> = LEAVES.iter().map(|leaf| leaf.path).collect();
        let before = paths.len();
        paths.sort_unstable();
        paths.dedup();
        assert_eq!(
            paths.len(),
            before,
            "строка листа должна быть одна: вторая молча затеняется поиском"
        );
        let stale: Vec<&str> = LEAVES
            .iter()
            .map(|leaf| leaf.path)
            .filter(|path| !declared.iter().any(|declared| declared == path))
            .collect();
        assert!(stale.is_empty(), "строки без листа в дереве: {stale:?}");
    }

    // Состав таблицы, общий с `tests/cli_global_flags.rs`.
    include!("global_flags_expected.in");

    fn paths_of(kept: impl Fn(&Leaf) -> bool) -> Vec<&'static str> {
        let mut paths: Vec<&str> = LEAVES
            .iter()
            .filter(|leaf| kept(leaf))
            .map(|leaf| leaf.path)
            .collect();
        // Сравнение по составу, а не по порядку: перестановка строк в таблице ничего не
        // меняет, а двойник ловится проверкой уникальности выше.
        paths.sort_unstable();
        paths
    }

    fn expected(paths: &[&'static str]) -> Vec<&'static str> {
        let mut paths: Vec<&'static str> = paths.to_vec();
        paths.sort_unstable();
        paths
    }

    /// Лист, объявивший поведение по глобальному ключу, обязан это поведение ещё и
    /// показать: отказы держат проверки в `tests/cli_global_flags.rs`. Обе половины
    /// сверяются с одним включаемым файлом, поэтому новый лист ломает эту проверку, а
    /// потерянная строка — ту.
    #[test]
    fn the_leaves_declaring_each_global_key_are_the_ones_the_shared_list_names() {
        assert_eq!(
            paths_of(|leaf| matches!(leaf.preview, Preview::Absent(_))),
            expected(LEAVES_WITHOUT_PREVIEW)
        );
        assert_eq!(
            paths_of(|leaf| matches!(leaf.base, Base::Ignores)),
            expected(LEAVES_IGNORING_THE_BASE)
        );
        assert_eq!(
            paths_of(|leaf| matches!(leaf.base, Base::Declares)),
            expected(LEAVES_DECLARING_THE_BASE)
        );
        assert_eq!(
            paths_of(|leaf| matches!(leaf.preview, Preview::Runs)),
            expected(LEAVES_WITH_PREVIEW)
        );
    }
}

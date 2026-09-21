//! Человеческая поверхность раннера: лента узлов и их подробностей.
//!
//! Форма закреплена `docs/schemas/text-output.json` и разобрана записью
//! `CTR.CLI.TEXT-OUTPUT`. Здесь живёт весь её словарь: знаки статуса, отступ
//! подробностей, виды строк и порядок, в котором подробности печатаются.
//!
//! Два правила держат эту поверхность читаемой:
//!
//! 1. Статус несёт знак, а не цвет. Цвет — оформление: он пропадает в файле журнала,
//!    в CI и у человека, который его не различает, и вывод обязан остаться понятным.
//! 2. Порядок подробностей один и тот же у всех команд: сначала что это, потом что
//!    произошло, потом что получилось, и только в конце — что не так.

use crate::command_envelope::Envelope;
use serde::Serialize;
#[cfg(test)]
use serde_json::{json, Value};

/// Знак узла. Меняется вместе с версией формы, а не вместе с настроением.
pub const MARK_SUCCEEDED: &str = "●";
pub const MARK_WARNED: &str = "▲";
pub const MARK_FAILED: &str = "✖";
pub const MARK_RUNNING: &str = "◌";
pub const MARK_SKIPPED: &str = "○";

/// Знак узла живого прогресса.
///
/// Словарь общий с итоговой лентой: один и тот же знак значит одно и то же, пока идёт
/// работа и когда она кончилась. Раньше живой прогресс рисовал `●` для всех четырёх
/// состояний и различал их только цветом.
pub fn progress_mark(status: &str) -> (&'static str, &'static str) {
    match status {
        "failed" => (MARK_FAILED, "31"),
        "running" => (MARK_RUNNING, "36"),
        "skipped" => (MARK_SKIPPED, "90"),
        _ => (MARK_SUCCEEDED, "32"),
    }
}

const DETAIL_INDENT: &str = "   ";

/// Словарь формы и путь её артефакта существуют ради проверки
/// `generated_text_output_grammar_is_current`, которую называет `check:` у активного
/// контракта `CTR.CLI.TEXT-OUTPUT`. В рантайме их никто не зовёт, поэтому они собираются
/// только под тестами — удалить их нельзя, это владелец `docs/schemas/text-output.json`.
#[cfg(test)]
const PIPE: &str = "│";
#[cfg(test)]
const ERROR_PREFIX: &str = "ERROR: ";
#[cfg(test)]
pub const TEXT_OUTPUT_GRAMMAR_PATH: &str = "docs/schemas/text-output.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineStatus {
    Succeeded,
    Failed,
}

/// Что показывает знак узла.
///
/// `Warned` не приходит от вызывающего: узел, у которого среди подробностей есть
/// предупреждение, показывается предупреждением сам. Иначе знак и содержимое
/// расходятся ровно там, где читатель им верит.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeMark {
    Succeeded,
    Warned,
    Failed,
}

impl NodeMark {
    const fn glyph(self) -> &'static str {
        match self {
            Self::Succeeded => MARK_SUCCEEDED,
            Self::Warned => MARK_WARNED,
            Self::Failed => MARK_FAILED,
        }
    }

    /// Слово исхода для подписи узла. Знак и слово берутся из одного значения, поэтому
    /// разойтись не могут.
    const fn word(self) -> &'static str {
        match self {
            Self::Succeeded => "completed successfully",
            Self::Warned => "completed with warnings",
            Self::Failed => "failed",
        }
    }

    const fn color(self) -> &'static str {
        match self {
            Self::Succeeded => "32",
            Self::Warned => "33",
            Self::Failed => "31",
        }
    }
}

/// Вид строки подробности. Порядок объявления — порядок печати.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DetailKind {
    /// `ключ: значение` — из чего состоял запрос и куда он направлен.
    Fact,
    /// Свободная строка: то, что не разложилось ни на ключ, ни на знак.
    Free,
    /// Шаг со своим исходом: `✓` сделано, `✗` не вышло, `○` пропущено, `→ ` запланировано.
    Mark,
    /// `[artifact] …` — что легло на диск.
    Artifact,
    /// `[diagnostic] …`, `[detail] …` и прочие пометки-улики.
    Note,
    /// `[warning] …` — сделано, но не так, как просили.
    Warning,
    /// `[error] …`, `[error:код] …` — не сделано.
    Error,
}

const STEP_MARKS: [&str; 4] = ["✓ ", "✗ ", "○ ", "→ "];

fn detail_kind(line: &str) -> DetailKind {
    if let Some((label, _)) = bracketed_prefix(line) {
        return match label {
            "[warning]" => DetailKind::Warning,
            "[artifact]" => DetailKind::Artifact,
            _ if label == "[error]" || label.starts_with("[error:") => DetailKind::Error,
            _ => DetailKind::Note,
        };
    }
    if STEP_MARKS.iter().any(|mark| line.starts_with(mark)) {
        return DetailKind::Mark;
    }
    if is_fact(line) {
        return DetailKind::Fact;
    }
    DetailKind::Free
}

/// `ключ: значение`, где ключ — одно слово без пробелов и двоеточий.
fn is_fact(line: &str) -> bool {
    match line.split_once(": ") {
        Some((key, _)) => !key.is_empty() && !key.contains(char::is_whitespace),
        None => false,
    }
}

/// Форма, порождённая из словаря выше.
///
/// Артефакт не пишется руками: пока он совпадает с этим словарём, описание формы и
/// её исполнение — одно и то же. Собирается только под тестами: единственный вызывающий —
/// контрактная проверка `generated_text_output_grammar_is_current`.
#[cfg(test)]
pub fn text_output_grammar() -> Value {
    json!({
        "_comment": "Порождается UPDATE_TEXT_OUTPUT_GRAMMAR=1 cargo test generated_text_output_grammar_is_current; руками не правится.",
        "version": 1,
        "streams": {
            "stdout": ["node", "detail", "separator", "bare"],
            "stderr": ["error"]
        },
        "node_marks": {
            "succeeded": MARK_SUCCEEDED,
            "warned": MARK_WARNED,
            "failed": MARK_FAILED,
            "running": MARK_RUNNING,
            "skipped": MARK_SKIPPED
        },
        "line_kinds": {
            "node": {
                "pattern": format!(
                    "^[{MARK_SUCCEEDED}{MARK_WARNED}{MARK_FAILED}{MARK_RUNNING}{MARK_SKIPPED}] \\S.*$"
                ),
                "description": "Узел ленты: знак статуса и подпись. Знак читается без цвета."
            },
            "detail": {
                "pattern": format!("^{PIPE}{DETAIL_INDENT}\\S.*$"),
                "description": "Подробность узла."
            },
            "separator": {
                "pattern": format!("^{PIPE}$"),
                "description": "Пустая связка между узлами."
            },
            "bare": {
                "pattern": "^v8-runner \\S+$",
                "description": "Ответ без ленты. Так отвечает только `version`."
            },
            "error": {
                "pattern": format!("^{ERROR_PREFIX}\\S.*$"),
                "description": "Отказ в stderr — последнее слово команды."
            }
        },
        "detail_kinds": [
            {"kind": "fact", "pattern": "^[^\\s:]+: .*$", "description": "ключ: значение"},
            {"kind": "free", "pattern": "^.+$", "description": "свободная строка"},
            {"kind": "mark", "pattern": "^[✓✗○→] .*$", "description": "шаг со своим исходом"},
            {"kind": "artifact", "pattern": "^\\[artifact\\] .*$", "description": "что легло на диск"},
            {"kind": "note", "pattern": "^\\[[a-z][a-z0-9_-]*\\] .*$", "description": "пометка-улика"},
            {"kind": "warning", "pattern": "^\\[warning\\] .*$", "description": "сделано не так, как просили"},
            {"kind": "error", "pattern": "^\\[error(:[a-z0-9_]+)?\\] .*$", "description": "не сделано"}
        ]
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineItem {
    pub status: TimelineStatus,
    pub label: TimelineLabel,
    pub detail: Option<String>,
}

/// Подпись узла: либо названа целиком, либо собирается из предмета.
///
/// Во втором случае слово исхода выбирает presenter — тем же правилом, каким выбирает
/// знак. Пока слово собирал рендерер, оно расходилось со знаком: у `syntax` подпись
/// называла проверку успешной, имея предупреждение среди подробностей, у `test` —
/// обещала предупреждения, не имея их. Оба случая находились тестами, а не правилом.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimelineLabel {
    /// Готовая подпись: у узла нет стандартного исхода («Dump skipped: …», «config:»).
    Fixed(String),
    /// Предмет узла: слово исхода подставит presenter.
    Subject(String),
}

impl TimelineItem {
    pub fn new(status: TimelineStatus, label: impl Into<String>) -> Self {
        Self {
            status,
            label: TimelineLabel::Fixed(label.into()),
            detail: None,
        }
    }

    /// Узел со стандартным исходом: рендерер называет предмет, слово выбирает presenter.
    pub fn outcome(status: TimelineStatus, subject: impl Into<String>) -> Self {
        Self {
            status,
            label: TimelineLabel::Subject(subject.into()),
            detail: None,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

pub struct TextPresenter {
    pub no_color: bool,
}

impl TextPresenter {
    pub fn print_error(&self, msg: &str) {
        if self.no_color {
            eprintln!("ERROR: {msg}");
        } else {
            eprintln!("\x1b[31mERROR\x1b[0m: {msg}");
        }
    }

    pub fn print_timeline(&self, items: &[TimelineItem]) {
        for (index, item) in items.iter().enumerate() {
            let last = index + 1 == items.len();
            let details = ordered_details(item);
            let mark = node_mark(item.status, &details);
            let label = match &item.label {
                TimelineLabel::Fixed(label) => label.clone(),
                TimelineLabel::Subject(subject) => format!("{subject} {}", mark.word()),
            };
            println!("{} {}", self.timeline_node(mark), label);

            let prefix = self.timeline_pipe();
            for line in &details {
                println!("{prefix}{DETAIL_INDENT}{}", self.timeline_detail(line));
            }

            if !last {
                println!("{}", self.timeline_pipe());
            }
        }
    }

    /// Ответ без ленты. Так отвечает только `version`: заворачивать одну строку в
    /// узел значит мешать тому, кто её читает подстановкой в другую команду.
    pub fn print_bare(&self, line: &str) {
        println!("{line}");
    }

    /// Узел, предваряющий ленту команды: печатается со связкой после себя, чтобы лента
    /// команды продолжила его, а не прижалась к нему.
    pub fn print_leading_node(&self, item: &TimelineItem) {
        self.print_timeline(std::slice::from_ref(item));
        println!("{}", self.timeline_pipe());
    }

    fn timeline_node(&self, mark: NodeMark) -> String {
        let glyph = mark.glyph();
        if self.no_color {
            glyph.to_owned()
        } else {
            format!("\x1b[{}m{glyph}\x1b[0m", mark.color())
        }
    }

    fn timeline_pipe(&self) -> String {
        if self.no_color {
            "│".to_owned()
        } else {
            "\x1b[34m│\x1b[0m".to_owned()
        }
    }

    fn timeline_detail(&self, detail: &str) -> String {
        if self.no_color {
            return detail.to_owned();
        }

        if let Some((prefix, rest)) = bracketed_prefix(detail) {
            return format!("\x1b[1;34m{prefix}\x1b[0m{rest}");
        }

        if let Some(rest) = detail.strip_prefix("Изменения:") {
            return format!("\x1b[1;34mИзменения\x1b[0m:{rest}");
        }

        if let Some(rest) = detail.strip_prefix("✓ ") {
            return format!("\x1b[1;32m✓\x1b[0m {rest}");
        }

        if let Some(rest) = detail.strip_prefix("✗ ") {
            return format!("\x1b[1;31m✗\x1b[0m {rest}");
        }

        if let Some(rest) = detail.strip_prefix("○ ") {
            return format!("\x1b[90m○\x1b[0m {rest}");
        }

        detail.to_owned()
    }
}

/// Подробности узла в порядке видов: сначала предмет, в конце — то, что не так.
///
/// Сортировка устойчива, поэтому внутри вида сохраняется порядок, в котором строки
/// собрал рендерер: у шагов он и есть порядок выполнения.
fn ordered_details(item: &TimelineItem) -> Vec<String> {
    let mut lines: Vec<String> = item
        .detail
        .as_deref()
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect();
    lines.sort_by_key(|line| detail_kind(line));
    drop_repeated_messages(&mut lines);
    lines
}

/// Убирает повтор одного и того же текста под разными пометками.
///
/// Одно и то же сообщение приходило и уликой, и предупреждением, и отказом — читатель
/// видел его трижды и искал разницу, которой нет. Остаётся последнее вхождение: виды
/// уже отсортированы по возрастанию серьёзности, поэтому последняя пометка и есть самая
/// точная (`[error:код]` вместо `[error]`, `[warning]` вместо `[diagnostic]`).
fn drop_repeated_messages(lines: &mut Vec<String>) {
    let mut seen_later = std::collections::HashSet::new();
    let mut kept: Vec<String> = Vec::with_capacity(lines.len());
    for line in lines.iter().rev() {
        let Some((_, message)) = bracketed_prefix(line) else {
            kept.push(line.clone());
            continue;
        };
        if seen_later.insert(message.trim().to_owned()) {
            kept.push(line.clone());
        }
    }
    kept.reverse();
    *lines = kept;
}

/// Знак узла: отказ называет вызывающий, предупреждение узел находит у себя сам.
///
/// Всплывшая, но не фатальная ошибка среди подробностей — тоже предупреждение: узел,
/// в котором есть `[error…]`, не показывается безоблачным только потому, что команда
/// в целом дошла до конца.
fn node_mark(status: TimelineStatus, details: &[String]) -> NodeMark {
    if matches!(status, TimelineStatus::Failed) {
        return NodeMark::Failed;
    }
    if details
        .iter()
        .map(|line| detail_kind(line))
        .any(|kind| matches!(kind, DetailKind::Warning | DetailKind::Error))
    {
        return NodeMark::Warned;
    }
    NodeMark::Succeeded
}

fn bracketed_prefix(value: &str) -> Option<(&str, &str)> {
    if !value.starts_with('[') {
        return None;
    }
    let prefix_end = value.find(']')? + 1;
    Some(value.split_at(prefix_end))
}

pub struct JsonPresenter;

impl JsonPresenter {
    pub fn print<T: Serialize>(&self, envelope: &Envelope<T>) {
        match serde_json::to_string_pretty(envelope) {
            Ok(s) => println!("{s}"),
            Err(e) => eprintln!("JSON serialization error: {e}"),
        }
    }

    /// Конверт, уже собранный как значение: так печатается ответ, к которому presenter
    /// дописал предупреждения загрузки.
    pub fn print_value(&self, envelope: &serde_json::Value) {
        match serde_json::to_string_pretty(envelope) {
            Ok(s) => println!("{s}"),
            Err(e) => eprintln!("JSON serialization error: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TextPresenter;

    #[test]
    fn timeline_detail_highlights_status_markers() {
        let presenter = TextPresenter { no_color: false };

        assert_eq!(
            presenter.timeline_detail("✓ completed"),
            "\x1b[1;32m✓\x1b[0m completed"
        );
        assert_eq!(
            presenter.timeline_detail("✗ failed"),
            "\x1b[1;31m✗\x1b[0m failed"
        );
        assert_eq!(
            presenter.timeline_detail("○ skipped"),
            "\x1b[90m○\x1b[0m skipped"
        );
    }

    /// Словарь формы и её артефакт — одно и то же, пока эта проверка зелёная.
    #[test]
    fn generated_text_output_grammar_is_current() {
        let generated = crate::command_data::schema_json_pretty(&super::text_output_grammar());
        if std::env::var_os("UPDATE_TEXT_OUTPUT_GRAMMAR").is_some() {
            std::fs::write(super::TEXT_OUTPUT_GRAMMAR_PATH, &generated).expect("write grammar");
        }
        let actual =
            std::fs::read_to_string(super::TEXT_OUTPUT_GRAMMAR_PATH).expect("grammar artefact");
        assert_eq!(
            actual, generated,
            "{} is stale; rerun UPDATE_TEXT_OUTPUT_GRAMMAR=1 cargo test generated_text_output_grammar_is_current",
            super::TEXT_OUTPUT_GRAMMAR_PATH
        );
    }

    /// Подробности печатаются по видам: предмет первым, отказ последним.
    #[test]
    fn details_are_printed_subject_first_and_trouble_last() {
        let item = super::TimelineItem::new(super::TimelineStatus::Succeeded, "dump")
            .with_detail(
                "[error:dump_failed] no\n[warning] slow\n✓ done\n[artifact] out.cf\nmode: full\n[diagnostic] log -> a.log",
            );
        let ordered = super::ordered_details(&item);
        assert_eq!(
            ordered,
            vec![
                "mode: full",
                "✓ done",
                "[artifact] out.cf",
                "[diagnostic] log -> a.log",
                "[warning] slow",
                "[error:dump_failed] no",
            ]
        );
    }

    /// Знак узла виден без цвета, и предупреждение узел находит у себя сам.
    #[test]
    fn a_node_mark_is_readable_without_colour() {
        let plain = super::TimelineItem::new(super::TimelineStatus::Succeeded, "ok");
        let warned = super::TimelineItem::new(super::TimelineStatus::Succeeded, "ok")
            .with_detail("[warning] slow");
        let failed =
            super::TimelineItem::new(super::TimelineStatus::Failed, "no").with_detail("[error] no");

        let mark = |item: &super::TimelineItem| {
            super::node_mark(item.status, &super::ordered_details(item)).glyph()
        };
        assert_eq!(mark(&plain), super::MARK_SUCCEEDED);
        assert_eq!(mark(&warned), super::MARK_WARNED);
        assert_eq!(mark(&failed), super::MARK_FAILED);
        assert_ne!(super::MARK_SUCCEEDED, super::MARK_FAILED);
    }

    #[test]
    fn timeline_detail_keeps_no_color_plain() {
        let presenter = TextPresenter { no_color: true };

        assert_eq!(presenter.timeline_detail("✓ completed"), "✓ completed");
    }

    /// Подпись узла и его знак берутся из одного значения: предупреждение среди
    /// подробностей меняет и слово, и знак. Фальсификатор правила — тот же узел без
    /// предупреждения обязан называться успешным.
    #[test]
    fn the_word_of_an_outcome_follows_the_sign() {
        let clean = super::TimelineItem::outcome(super::TimelineStatus::Succeeded, "Dump")
            .with_detail("mode: full");
        let warned = super::TimelineItem::outcome(super::TimelineStatus::Succeeded, "Dump")
            .with_detail("[warning] slow");
        let failed = super::TimelineItem::outcome(super::TimelineStatus::Failed, "Dump")
            .with_detail("[error] no");

        let word = |item: &super::TimelineItem| {
            let details = super::ordered_details(item);
            super::node_mark(item.status, &details).word().to_owned()
        };
        let mark = |item: &super::TimelineItem| {
            let details = super::ordered_details(item);
            super::node_mark(item.status, &details).glyph()
        };

        assert_eq!(word(&clean), "completed successfully");
        assert_eq!(mark(&clean), super::MARK_SUCCEEDED);
        assert_eq!(word(&warned), "completed with warnings");
        assert_eq!(mark(&warned), super::MARK_WARNED);
        assert_eq!(word(&failed), "failed");
        assert_eq!(mark(&failed), super::MARK_FAILED);
    }
}

//! Поток событий хода команды на stdout при `--events -` (`CTR.WIRE.PROGRESS-EVENTS`).
//!
//! С этим ключом stdout — NDJSON: события хода, каждое строкой, и последней строкой
//! итоговый конверт. Всё, что пишется в stdout, пока поток открыт, проходит через один замок
//! этого модуля: событие из потока сигнала не врежется в середину конверта, а после конверта
//! ничего не пишется. Без ключа поток закрыт, и stdout прежний.

use std::io::Write as _;
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde::Serialize;

use crate::domain::progress::{AgentProgressKind, CancelReason, ProgressPhase};
use crate::support::path::response_json;

/// Версия формы событий.
pub const EVENTS_VERSION: u32 = 1;

#[cfg(test)]
pub const PROGRESS_EVENT_SCHEMA_PATH: &str = "docs/schemas/progress-event.schema.json";

/// Одно событие хода. Ключи идут в порядке формы: `version`, `event`, `at`, `command`,
/// дальше поля события.
#[derive(Debug, Serialize, schemars::JsonSchema)]
#[schemars(title = "v8-runner progress event")]
pub struct ProgressEvent<'a> {
    #[schemars(range(min = 1, max = 1))]
    version: u32,
    #[serde(flatten)]
    event: Event<'a>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(tag = "event", rename_all = "snake_case")]
enum Event<'a> {
    /// Команда начала шаг.
    Step {
        #[serde(flatten)]
        head: Head<'a>,
        phase: ProgressPhase,
        /// Набор исходников шага; `null`, если шаг не о наборе.
        #[schemars(required, extend("type" = ["string", "null"]))]
        set: Option<&'a str>,
        /// Номер шага с единицы; растёт на единицу с каждым шагом.
        #[schemars(range(min = 1))]
        step_index: u32,
        /// Сколько шагов известно сейчас: не меньше `step_index` и может расти.
        #[schemars(range(min = 1))]
        step_count: u32,
    },
    /// Промежуточное сообщение агента Конфигуратора.
    Agent {
        #[serde(flatten)]
        head: Head<'a>,
        agent_type: AgentProgressKind,
        /// Текст сообщения с маской учётных данных.
        text: &'a str,
    },
    /// Команду отменили: пишется один раз, при первом сигнале.
    Cancel {
        #[serde(flatten)]
        head: Head<'a>,
        reason: CancelReason,
    },
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct Head<'a> {
    /// Мгновение события, RFC 3339 в UTC.
    at: String,
    /// Имя команды, как его называет итоговый конверт.
    command: &'a str,
}

#[derive(Default)]
struct Stream {
    /// Имя команды; `None` — поток не открыт.
    command: Option<String>,
    step_index: u32,
    cancelled: bool,
    /// Конверт напечатан: больше ничего не пишется.
    closed: bool,
}

static STREAM: Mutex<Stream> = Mutex::new(Stream {
    command: None,
    step_index: 0,
    cancelled: false,
    closed: false,
});

fn stream() -> MutexGuard<'static, Stream> {
    STREAM.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Открывает поток событий команды `command`: с этого момента stdout пишет только этот
/// модуль.
pub fn open(command: &str) {
    let mut stream = stream();
    *stream = Stream {
        command: Some(command.to_owned()),
        ..Stream::default()
    };
}

/// Открыт ли поток: тогда конверт печатается последней строкой через [`finish`].
pub fn is_open() -> bool {
    stream().command.is_some()
}

/// Шаг команды.
pub fn step(phase: ProgressPhase, set: Option<&str>) {
    let mut stream = stream();
    if let Some(line) = stream.step(phase, set) {
        write_line(&line);
    }
}

/// Промежуточное сообщение агента. Текст уже с маской.
pub fn agent(agent_type: AgentProgressKind, text: &str) {
    let stream = stream();
    if let Some(line) = stream.agent(agent_type, text) {
        write_line(&line);
    }
}

/// Отмена команды: только первая, повторный сигнал события не пишет.
pub fn cancel(reason: CancelReason) {
    let mut stream = stream();
    if let Some(line) = stream.cancel(reason) {
        write_line(&line);
    }
}

/// Итоговый конверт одной строкой. После него поток закрыт: второй конверт и опоздавшие
/// события не пишутся.
pub fn finish(envelope_line: &str) {
    let mut stream = stream();
    if stream.finish() {
        write_line(envelope_line);
    }
}

/// Строки потока решает поток, пишут их функции выше — под тем же замком.
impl Stream {
    fn writable(&self) -> Option<&str> {
        if self.closed {
            return None;
        }
        self.command.as_deref()
    }

    fn step(&mut self, phase: ProgressPhase, set: Option<&str>) -> Option<String> {
        let index = self.step_index.saturating_add(1);
        let line = render(Event::Step {
            head: Head::now(self.writable()?),
            phase,
            set,
            step_index: index,
            step_count: index,
        });
        self.step_index = index;
        Some(line)
    }

    fn agent(&self, agent_type: AgentProgressKind, text: &str) -> Option<String> {
        Some(render(Event::Agent {
            head: Head::now(self.writable()?),
            agent_type,
            text,
        }))
    }

    fn cancel(&mut self, reason: CancelReason) -> Option<String> {
        if self.cancelled {
            return None;
        }
        let line = render(Event::Cancel {
            head: Head::now(self.writable()?),
            reason,
        });
        self.cancelled = true;
        Some(line)
    }

    /// Закрывает поток; `true` — конверт пора печатать.
    fn finish(&mut self) -> bool {
        if self.writable().is_none() {
            return false;
        }
        self.closed = true;
        true
    }
}

impl<'a> Head<'a> {
    fn now(command: &'a str) -> Self {
        Self {
            at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            command,
        }
    }
}

fn render(event: Event<'_>) -> String {
    serde_json::to_string(&ProgressEvent {
        version: EVENTS_VERSION,
        event,
    })
    .expect("a progress event serializes")
}

/// Строка в stdout сразу: зовётся только под замком потока.
fn write_line(line: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{}", response_json(line));
    let _ = out.flush();
}

#[cfg(test)]
pub(crate) fn generated_schema() -> serde_json::Value {
    let mut schema = crate::command_data::generated_schema(
        schemars::schema_for!(ProgressEvent<'static>),
        "progress-event",
    );
    schema["$id"] = serde_json::json!(format!(
        "{}/progress-event.schema.json",
        crate::command_data::REPOSITORY_RAW_SCHEMA_ROOT
    ));
    schema["title"] = serde_json::json!("v8-runner progress event");
    // Ветки событий закрыты, а `version` лежит снаружи — в корне его не пустила бы ни одна
    // ветка. Поле переезжает в каждую ветку первым, как оно и печатается.
    let object = schema.as_object_mut().expect("root schema object");
    let shared = object.remove("properties").expect("shared properties");
    let shared_required = object.remove("required").expect("shared required");
    for branch in object
        .get_mut("oneOf")
        .and_then(serde_json::Value::as_array_mut)
        .expect("one branch per event")
    {
        let properties = branch["properties"]
            .as_object_mut()
            .expect("branch properties");
        for (name, property) in shared.as_object().expect("shared properties object") {
            properties.insert(name.clone(), property.clone());
        }
        let required = branch["required"].as_array_mut().expect("branch required");
        for (position, name) in shared_required
            .as_array()
            .expect("shared required list")
            .iter()
            .enumerate()
        {
            required.insert(position, name.clone());
        }
    }
    schema
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(event: Event<'_>) -> serde_json::Value {
        let line = render(event);
        assert!(
            line.starts_with(r#"{"version":1,"event":""#),
            "the form opens with its version and its event: {line}"
        );
        let at = line.find(r#","at":""#).expect("at");
        let command = line.find(r#","command":""#).expect("command");
        assert!(at < command, "at comes before command: {line}");
        serde_json::from_str(&line).unwrap()
    }

    fn head() -> Head<'static> {
        Head::now("pull")
    }

    /// Шаг растёт на единицу, отмена пишется один раз, а после конверта поток молчит.
    #[test]
    fn a_stream_counts_its_steps_cancels_once_and_falls_silent_after_the_envelope() {
        let mut stream = Stream::default();
        assert!(stream.step(ProgressPhase::Dump, None).is_none(), "not open");
        assert!(!stream.finish(), "not open");
        stream.command = Some("pull".to_owned());
        let indices: Vec<(u64, u64)> = (0..3)
            .map(|_| {
                let line: serde_json::Value =
                    serde_json::from_str(&stream.step(ProgressPhase::Load, Some("main")).unwrap())
                        .unwrap();
                (
                    line["step_index"].as_u64().unwrap(),
                    line["step_count"].as_u64().unwrap(),
                )
            })
            .collect();
        assert_eq!(indices, [(1, 1), (2, 2), (3, 3)]);
        assert!(stream.cancel(CancelReason::Cancelled).is_some());
        assert!(stream.cancel(CancelReason::Cancelled).is_none(), "once");
        assert!(
            stream
                .agent(AgentProgressKind::Log, "after cancel")
                .is_some(),
            "the agent may still speak after a cancellation"
        );
        assert!(stream.finish(), "the envelope is printed");
        assert!(!stream.finish(), "a second envelope is not");
        assert!(stream.step(ProgressPhase::Load, None).is_none());
        assert!(stream.agent(AgentProgressKind::Log, "late").is_none());
    }

    #[test]
    fn generated_progress_event_schema_is_current() {
        let generated = crate::command_data::schema_json_pretty(&generated_schema());
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(PROGRESS_EVENT_SCHEMA_PATH);
        if std::env::var_os("UPDATE_PROGRESS_EVENT_SCHEMA").is_some() {
            std::fs::write(&path, &generated).expect("write progress event schema");
        }
        assert_eq!(
            std::fs::read_to_string(path).expect("progress event schema artifact"),
            generated,
            "{PROGRESS_EVENT_SCHEMA_PATH} is stale; rerun UPDATE_PROGRESS_EVENT_SCHEMA=1 cargo test generated_progress_event_schema_is_current"
        );
    }

    /// Каждое событие проходит схему, ключи идут в порядке формы, а лишнее поле и чужая
    /// фаза схему не проходят.
    #[test]
    fn every_progress_event_passes_the_schema_and_nothing_else_does() {
        let validator = jsonschema::validator_for(&generated_schema()).expect("schema");
        let events = [
            event(Event::Step {
                head: head(),
                phase: ProgressPhase::Dump,
                set: Some("main"),
                step_index: 1,
                step_count: 1,
            }),
            event(Event::Step {
                head: head(),
                phase: ProgressPhase::Session,
                set: None,
                step_index: 2,
                step_count: 2,
            }),
            event(Event::Agent {
                head: head(),
                agent_type: AgentProgressKind::LoadingIssue,
                text: "Invalid data path",
            }),
            event(Event::Cancel {
                head: head(),
                reason: CancelReason::Cancelled,
            }),
        ];
        for value in &events {
            assert!(validator.is_valid(value), "valid event: {value}");
            let mut extra = value.clone();
            extra["other"] = serde_json::json!(true);
            assert!(!validator.is_valid(&extra), "unknown field: {extra}");
            let mut version = value.clone();
            version["version"] = serde_json::json!(2);
            assert!(!validator.is_valid(&version), "unknown version: {version}");
        }
        let mut phase = events[0].clone();
        phase["phase"] = serde_json::json!("other");
        assert!(!validator.is_valid(&phase), "unknown phase: {phase}");
        let schema_phases = generated_schema()["$defs"]["ProgressPhase"]["oneOf"]
            .as_array()
            .map(Vec::len);
        assert_eq!(schema_phases, Some(ProgressPhase::ALL.len()));
    }
}

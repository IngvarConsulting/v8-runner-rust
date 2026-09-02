use std::fs::{File, OpenOptions};
use std::io::{self, Write as IoWrite};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::{Local, Utc};
use thiserror::Error;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::fmt::format::{FormatEvent, FormatFields, Writer};
use tracing_subscriber::fmt::time::FormatTime;
use tracing_subscriber::fmt::FmtContext;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::EnvFilter;

const ACTION_LOG_FILE_ENV: &str = "V8TR_ACTION_LOG_FILE";
const LIVE_PROGRESS_FILTER_DIRECTIVE: &str = "v8_runner::live_progress=info";

#[derive(Debug, Error)]
pub enum LoggingInitError {
    #[error("failed to open action log file '{path}': {source}")]
    OpenFile { path: PathBuf, source: io::Error },

    #[error("failed to initialize action logger: {0}")]
    Install(String),
}

pub fn init_action_logging(
    level: &str,
    output_format: &str,
    color_enabled: bool,
    work_path: &Path,
) -> Result<Option<PathBuf>, LoggingInitError> {
    init_action_logging_impl(level, output_format, color_enabled, work_path, false)
}

pub fn init_action_logging_deferred(
    level: &str,
    output_format: &str,
    color_enabled: bool,
    work_path: &Path,
) -> Result<Option<PathBuf>, LoggingInitError> {
    init_action_logging_impl(level, output_format, color_enabled, work_path, true)
}

fn init_action_logging_impl(
    level: &str,
    output_format: &str,
    color_enabled: bool,
    work_path: &Path,
    defer_file_open: bool,
) -> Result<Option<PathBuf>, LoggingInitError> {
    let log_path = resolve_action_log_path(output_format, work_path);
    let writer = ActionLogMakeWriter {
        stdout_enabled: output_format == "text",
        file: if defer_file_open {
            None
        } else {
            open_log_file(log_path.as_deref())?
        },
        deferred_file: if defer_file_open {
            log_path.as_ref().map(|path| {
                Arc::new(DeferredActionLog {
                    path: path.clone(),
                    file: Mutex::new(None),
                })
            })
        } else {
            None
        },
    };
    let ansi_enabled = output_format == "text" && color_enabled;
    let env_filter = env_filter_with_live_progress(level);

    if output_format == "text" {
        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .with_writer(writer)
            .with_timer(UtcTimer)
            .with_ansi(ansi_enabled)
            .with_target(false)
            .event_format(CliEventFormatter::default())
            .try_init()
            .map_err(|error| LoggingInitError::Install(error.to_string()))?;
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .with_writer(writer)
            .with_timer(UtcTimer)
            .with_ansi(false)
            .with_target(false)
            .try_init()
            .map_err(|error| LoggingInitError::Install(error.to_string()))?;
    }

    Ok(log_path)
}

fn env_filter_with_live_progress(level: &str) -> EnvFilter {
    let trimmed = level.trim();
    let filter = if trimmed.is_empty() {
        LIVE_PROGRESS_FILTER_DIRECTIVE.to_owned()
    } else {
        format!("{trimmed},{LIVE_PROGRESS_FILTER_DIRECTIVE}")
    };
    EnvFilter::try_new(filter)
        .unwrap_or_else(|_| EnvFilter::new(format!("info,{LIVE_PROGRESS_FILTER_DIRECTIVE}")))
}

fn resolve_action_log_path(output_format: &str, work_path: &Path) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(ACTION_LOG_FILE_ENV) {
        return Some(PathBuf::from(path));
    }

    if output_format == "json" {
        return Some(work_path.join("logs").join("mcp").join("actions.log"));
    }

    None
}

fn open_log_file(path: Option<&Path>) -> Result<Option<Arc<Mutex<File>>>, LoggingInitError> {
    let Some(path) = path else {
        return Ok(None);
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| LoggingInitError::OpenFile {
            path: path.to_path_buf(),
            source,
        })?;
    }

    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|source| LoggingInitError::OpenFile {
            path: path.to_path_buf(),
            source,
        })?;

    Ok(Some(Arc::new(Mutex::new(file))))
}

#[derive(Clone)]
struct ActionLogMakeWriter {
    stdout_enabled: bool,
    file: Option<Arc<Mutex<File>>>,
    deferred_file: Option<Arc<DeferredActionLog>>,
}

struct ActionLogWriter {
    stdout_enabled: bool,
    file: Option<Arc<Mutex<File>>>,
    deferred_file: Option<Arc<DeferredActionLog>>,
}

struct DeferredActionLog {
    path: PathBuf,
    file: Mutex<Option<File>>,
}

struct UtcTimer;
#[derive(Default)]
struct CliEventFormatter {
    timeline: Arc<Mutex<TimelineRenderState>>,
}

#[derive(Default)]
struct TimelineRenderState {
    started: bool,
    last_label: Option<String>,
}

struct TimelineRender {
    new_node: bool,
    connector_before: bool,
}
#[derive(Default)]
struct EventFieldVisitor {
    message: Option<String>,
    fields: Vec<String>,
    timeline_label: Option<String>,
    timeline_status: Option<String>,
    timeline_detail: Option<String>,
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for ActionLogMakeWriter {
    type Writer = ActionLogWriter;

    fn make_writer(&'a self) -> Self::Writer {
        ActionLogWriter {
            stdout_enabled: self.stdout_enabled,
            file: self.file.clone(),
            deferred_file: self.deferred_file.clone(),
        }
    }
}

impl FormatTime for UtcTimer {
    fn format_time(
        &self,
        writer: &mut tracing_subscriber::fmt::format::Writer<'_>,
    ) -> std::fmt::Result {
        write!(writer, "{}", Utc::now().format("%H:%M:%S%.3f"))
    }
}

impl<S, N> FormatEvent<S, N> for CliEventFormatter
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> std::fmt::Result {
        let meta = event.metadata();
        let mut visitor = EventFieldVisitor::default();
        event.record(&mut visitor);

        if let Some(label) = visitor.timeline_label.as_deref() {
            return self.format_timeline_event(
                &mut writer,
                label,
                visitor.timeline_status.as_deref().unwrap_or("succeeded"),
                visitor.timeline_detail.as_deref(),
            );
        }

        write_timeline_pipe(&mut writer)?;
        write!(writer, " ")?;
        write!(writer, "{}  ", Utc::now().format("%H:%M:%S%.3f"))?;
        write_level(&mut writer, meta.level())?;

        if let Some(message) = visitor.message.as_deref() {
            write!(writer, " ")?;
            write_message(&mut writer, message)?;
        }

        for field in &visitor.fields {
            write!(writer, " {field}")?;
        }

        writeln!(writer)
    }
}

impl CliEventFormatter {
    fn format_timeline_event(
        &self,
        writer: &mut Writer<'_>,
        label: &str,
        status: &str,
        detail: Option<&str>,
    ) -> std::fmt::Result {
        let render = {
            let mut state = self.timeline.lock().map_err(|_| std::fmt::Error)?;
            let same_label = state.last_label.as_deref() == Some(label);
            let render = TimelineRender {
                new_node: !same_label,
                connector_before: state.started && !same_label,
            };
            state.started = true;
            state.last_label = Some(label.to_owned());
            render
        };

        if render.connector_before {
            write_timeline_pipe(writer)?;
            writeln!(writer)?;
        }

        if render.new_node {
            write_timeline_node(writer, status)?;
            write!(writer, " ")?;
            write_timeline_label(writer, label)?;
            writeln!(writer)?;
        }

        let running_started_at = (status == "running").then(running_started_at_detail);

        if let Some(detail) = detail.filter(|value| !value.trim().is_empty()) {
            for line in detail.lines() {
                write_timeline_pipe(writer)?;
                write!(writer, "   ")?;
                if let Some(started_at) = running_started_at.as_deref() {
                    write_timeline_timestamp(writer, started_at)?;
                    write!(writer, "  ")?;
                }
                write_timeline_detail(writer, line.trim())?;
                writeln!(writer)?;
            }
        } else if let Some(started_at) = running_started_at.as_deref() {
            write_timeline_pipe(writer)?;
            write!(writer, "   ")?;
            write_timeline_timestamp(writer, started_at)?;
            writeln!(writer)?;
        }

        Ok(())
    }
}

fn running_started_at_detail() -> String {
    Local::now().format("%H:%M:%S").to_string()
}

fn write_timeline_pipe(writer: &mut Writer<'_>) -> std::fmt::Result {
    if writer.has_ansi_escapes() {
        write!(writer, "\x1b[34m│\x1b[0m")
    } else {
        write!(writer, "│")
    }
}

fn write_timeline_node(writer: &mut Writer<'_>, status: &str) -> std::fmt::Result {
    if writer.has_ansi_escapes() {
        let color = match status {
            "failed" => "31",
            "running" => "36",
            "skipped" => "90",
            _ => "32",
        };
        write!(writer, "\x1b[{color}m●\x1b[0m")
    } else {
        write!(writer, "●")
    }
}

fn write_timeline_label(writer: &mut Writer<'_>, label: &str) -> std::fmt::Result {
    if !writer.has_ansi_escapes() {
        return write!(writer, "{label}");
    }

    let Some(prefix_end) = label.find(':') else {
        return write!(writer, "{label}");
    };
    let (source_set, rest) = label.split_at(prefix_end);
    if source_set.is_empty() {
        return write!(writer, "{label}");
    }

    write_highlighted_label(writer, source_set, "1")?;
    write!(writer, "{rest}")
}

fn write_timeline_detail(writer: &mut Writer<'_>, detail: &str) -> std::fmt::Result {
    if !writer.has_ansi_escapes() {
        return write!(writer, "{detail}");
    }

    if let Some((prefix, rest)) = bracketed_prefix(detail) {
        write!(writer, "\x1b[1;34m{prefix}\x1b[0m")?;
        return write!(writer, "{rest}");
    }

    if let Some(rest) = detail.strip_prefix("Изменения:") {
        write_highlighted_label(writer, "Изменения", "1;34")?;
        return write!(writer, ":{rest}");
    }

    if let Some(rest) = detail.strip_prefix("✓ ") {
        write_highlighted_label(writer, "✓", "1;32")?;
        return write!(writer, " {rest}");
    }

    if let Some(rest) = detail.strip_prefix("✗ ") {
        write_highlighted_label(writer, "✗", "1;31")?;
        return write!(writer, " {rest}");
    }

    if let Some(rest) = detail.strip_prefix("○ ") {
        write_highlighted_label(writer, "○", "90")?;
        return write!(writer, " {rest}");
    }

    write!(writer, "{detail}")
}

fn write_timeline_timestamp(writer: &mut Writer<'_>, timestamp: &str) -> std::fmt::Result {
    if writer.has_ansi_escapes() {
        return write!(writer, "\x1b[90m{timestamp}\x1b[0m");
    }

    write!(writer, "{timestamp}")
}

fn bracketed_prefix(value: &str) -> Option<(&str, &str)> {
    if !value.starts_with('[') {
        return None;
    }
    let prefix_end = value.find(']')? + 1;
    Some(value.split_at(prefix_end))
}

impl tracing::field::Visit for EventFieldVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_owned());
        } else if !self.record_timeline_field(field.name(), value.to_owned()) {
            self.fields.push(format!(r#"{}="{}""#, field.name(), value));
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{value:?}"));
        } else if !self.record_timeline_field(field.name(), normalize_debug_value(value)) {
            self.fields.push(format!("{}={value:?}", field.name()));
        }
    }
}

impl EventFieldVisitor {
    fn record_timeline_field(&mut self, name: &str, value: String) -> bool {
        match name {
            "timeline_label" => self.timeline_label = Some(value),
            "timeline_status" => self.timeline_status = Some(value),
            "timeline_detail" => self.timeline_detail = Some(value),
            _ => return false,
        }
        true
    }
}

fn normalize_debug_value(value: &dyn std::fmt::Debug) -> String {
    let rendered = format!("{value:?}");
    rendered
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(&rendered)
        .to_owned()
}

fn write_level(writer: &mut Writer<'_>, level: &Level) -> std::fmt::Result {
    let label = format!("{level:>5}");
    if writer.has_ansi_escapes() {
        let code = match *level {
            Level::ERROR => "1;31",
            Level::WARN => "1;33",
            Level::INFO => "1;32",
            Level::DEBUG => "1;36",
            Level::TRACE => "1;35",
        };
        write!(writer, "\x1b[{code}m{label}\x1b[0m")
    } else {
        write!(writer, "{label}")
    }
}

fn write_message(writer: &mut Writer<'_>, message: &str) -> std::fmt::Result {
    if message == "running interactive edt command" {
        if writer.has_ansi_escapes() {
            write!(writer, "running interactive edt ")?;
            return write_highlighted_label(writer, "command", "1;96");
        }
        return write!(writer, "{message}");
    }

    if message == "interactive edt command finished" {
        if writer.has_ansi_escapes() {
            write!(writer, "interactive edt ")?;
            return write_highlighted_label(writer, "command finished", "1;96");
        }
        return write!(writer, "{message}");
    }

    let Some(prefix_end) = message.find(']') else {
        return write!(writer, "{message}");
    };

    if !message.starts_with('[') {
        return write!(writer, "{message}");
    }

    let (prefix, rest) = message.split_at(prefix_end + 1);
    if writer.has_ansi_escapes() {
        write!(writer, "\x1b[1;34m{prefix}\x1b[0m")?;
        write_stage_tail(writer, rest)
    } else {
        write!(writer, "{message}")
    }
}

fn write_stage_tail(writer: &mut Writer<'_>, rest: &str) -> std::fmt::Result {
    if let Some((before, project_name)) = split_project_name_suffix(rest) {
        write!(writer, "{before}")?;
        write_highlighted_label(writer, project_name, "1;37")
    } else {
        write!(writer, "{rest}")
    }
}

fn split_project_name_suffix(rest: &str) -> Option<(&str, &str)> {
    let (before, tail) = rest.rsplit_once(": ")?;
    let project_name = tail.trim();
    if project_name.is_empty() || project_name.contains(' ') {
        return None;
    }

    Some((&rest[..before.len() + 2], project_name))
}

fn write_highlighted_label(
    writer: &mut Writer<'_>,
    text: &str,
    ansi_code: &str,
) -> std::fmt::Result {
    if writer.has_ansi_escapes() {
        write!(writer, "\x1b[{ansi_code}m{text}\x1b[0m")
    } else {
        write!(writer, "{text}")
    }
}

impl IoWrite for ActionLogWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.stdout_enabled {
            let mut stdout = io::stdout();
            stdout.write_all(buf)?;
            stdout.flush()?;
        }

        if let Some(file) = &self.file {
            let mut file = file
                .lock()
                .map_err(|_| io::Error::other("action log mutex poisoned"))?;
            file.write_all(buf)?;
            file.flush()?;
        }
        if let Some(deferred) = &self.deferred_file {
            deferred.write_all(buf)?;
        }

        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.stdout_enabled {
            io::stdout().flush()?;
        }

        if let Some(file) = &self.file {
            let mut file = file
                .lock()
                .map_err(|_| io::Error::other("action log mutex poisoned"))?;
            file.flush()?;
        }
        if let Some(deferred) = &self.deferred_file {
            deferred.flush()?;
        }

        Ok(())
    }
}

impl DeferredActionLog {
    fn with_file<T>(&self, operation: impl FnOnce(&mut File) -> io::Result<T>) -> io::Result<T> {
        let mut file = self
            .file
            .lock()
            .map_err(|_| io::Error::other("deferred action log mutex poisoned"))?;
        if file.is_none() {
            if let Some(parent) = self.path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            *file = Some(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.path)?,
            );
        }
        operation(file.as_mut().expect("deferred action log was opened"))
    }

    fn write_all(&self, buf: &[u8]) -> io::Result<()> {
        self.with_file(|file| {
            file.write_all(buf)?;
            file.flush()
        })
    }

    fn flush(&self) -> io::Result<()> {
        let mut file = self
            .file
            .lock()
            .map_err(|_| io::Error::other("deferred action log mutex poisoned"))?;
        match file.as_mut() {
            Some(file) => file.flush(),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::{Arc, Mutex};

    use tracing_subscriber::EnvFilter;

    use super::{CliEventFormatter, UtcTimer};

    #[derive(Clone)]
    struct BufferMakeWriter {
        buffer: Arc<Mutex<Vec<u8>>>,
    }

    struct BufferWriter {
        buffer: Arc<Mutex<Vec<u8>>>,
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for BufferMakeWriter {
        type Writer = BufferWriter;

        fn make_writer(&'a self) -> Self::Writer {
            BufferWriter {
                buffer: self.buffer.clone(),
            }
        }
    }

    impl io::Write for BufferWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.buffer
                .lock()
                .map_err(|_| io::Error::other("buffer mutex poisoned"))?
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn render_event(status: &str, detail: &str) -> String {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let writer = BufferMakeWriter {
            buffer: buffer.clone(),
        };
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::new("info"))
            .with_writer(writer)
            .with_timer(UtcTimer)
            .with_ansi(false)
            .with_target(false)
            .event_format(CliEventFormatter::default())
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(
                timeline_status = status,
                timeline_label = "test: enterprise run",
                timeline_detail = detail
            );
        });

        let bytes = buffer.lock().expect("buffer").clone();
        String::from_utf8(bytes).expect("utf8")
    }

    #[test]
    fn running_timeline_event_adds_start_time_before_detail() {
        let output = render_event("running", "[Enterprise] enterprise run");

        let detail = output.find("[Enterprise] enterprise run").expect("detail");
        let line = output
            .lines()
            .find(|line| line.contains("[Enterprise] enterprise run"))
            .expect("detail line");
        let time = line
            .split_whitespace()
            .find(|part| is_hh_mm_ss(part))
            .expect("start time");
        assert!(output.find(time).expect("start time") < detail);
        assert!(!output.contains("started_at: "));
    }

    #[test]
    fn non_running_timeline_event_does_not_add_start_timestamp() {
        let output = render_event("succeeded", "✓ completed");

        assert!(!output.contains("started_at: "));
        assert!(output.contains("✓ completed"));
    }

    fn is_hh_mm_ss(value: &str) -> bool {
        let bytes = value.as_bytes();
        bytes.len() == 8
            && bytes[0].is_ascii_digit()
            && bytes[1].is_ascii_digit()
            && bytes[2] == b':'
            && bytes[3].is_ascii_digit()
            && bytes[4].is_ascii_digit()
            && bytes[5] == b':'
            && bytes[6].is_ascii_digit()
            && bytes[7].is_ascii_digit()
    }
}

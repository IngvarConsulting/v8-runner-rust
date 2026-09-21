use crate::command_envelope::Envelope;
use crate::output::text::{JsonPresenter, TextPresenter, TimelineItem, TimelineStatus};
use serde::Serialize;

pub enum ColorMode {
    Enabled,
    Disabled,
}

pub struct Presenter {
    format: String,
    text: TextPresenter,
    json: JsonPresenter,
    /// Предупреждения загрузки конфига, которые JSON-конверт понесёт вместе с ответом
    /// команды. В тексте они печатаются сразу, своим узлом, и здесь не копятся.
    load_warnings: Vec<String>,
}

impl Presenter {
    pub fn new(format: String, color_mode: ColorMode) -> Self {
        let no_color = matches!(color_mode, ColorMode::Disabled);
        Self {
            format,
            text: TextPresenter { no_color },
            json: JsonPresenter,
            load_warnings: Vec::new(),
        }
    }

    pub fn is_json(&self) -> bool {
        self.format == "json"
    }

    /// Предупреждения, с которыми загрузился конфиг: в тексте — узел `▲ config: …`
    /// перед лентой команды, в JSON — хвост `warnings` любого конверта, который будет
    /// напечатан после. Пустой список ничего не печатает и ничего не запоминает.
    pub fn note_load_warnings(&mut self, config_path: &str, warnings: &[String]) {
        if warnings.is_empty() {
            return;
        }
        if self.is_json() {
            self.load_warnings.extend(warnings.iter().cloned());
            return;
        }
        let details = warnings
            .iter()
            .map(|warning| format!("[warning] {warning}"))
            .collect::<Vec<_>>()
            .join("\n");
        let node = TimelineItem::new(TimelineStatus::Succeeded, format!("config: {config_path}"))
            .with_detail(details);
        self.text.print_leading_node(&node);
    }

    pub fn print_error(&self, msg: &str) {
        if self.is_json() {
            let env =
                Envelope::<serde_json::Value>::err("error", 0, serde_json::json!({"message": msg}));
            self.print_envelope(&env);
        } else {
            self.text.print_error(msg);
        }
    }

    /// Однострочный ответ без ленты — форма, объявленная для `version`.
    pub fn print_bare(&self, line: &str) {
        if !self.is_json() {
            self.text.print_bare(line);
        }
    }

    pub fn print_timeline(&self, items: &[TimelineItem]) {
        if !self.is_json() {
            self.text.print_timeline(items);
        }
    }

    pub fn print_envelope<T: Serialize>(&self, envelope: &Envelope<T>) {
        if !self.is_json() {
            // text mode: callers render explicit timeline items.
            return;
        }
        if self.load_warnings.is_empty() {
            self.json.print(envelope);
            return;
        }
        // Предупреждения загрузки идут после предупреждений команды: команда говорит о
        // своём первой, а `config init` ждёт своё предупреждение первым. Конверт
        // собирается заново с теми же полями, чтобы порядок ключей не менялся.
        let mut warnings = envelope.warnings.clone();
        warnings.extend(self.load_warnings.iter().cloned());
        let merged = Envelope {
            ok: envelope.ok,
            command: envelope.command.clone(),
            duration_ms: envelope.duration_ms,
            data: &envelope.data,
            warnings,
            steps: envelope.steps.clone(),
            error: envelope.error.clone(),
        };
        self.json.print(&merged);
    }
}

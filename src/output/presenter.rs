use crate::command_envelope::Envelope;
use crate::domain::warning::Warning;
use crate::output::text::{JsonPresenter, TextPresenter, TimelineItem, TimelineStatus};
use serde::Serialize;
use std::cell::RefCell;

pub enum ColorMode {
    Enabled,
    Disabled,
}

pub struct Presenter {
    format: String,
    text: TextPresenter,
    json: JsonPresenter,
    /// Предупреждения до ответа команды — загрузки конфига, замка базы, — которые
    /// JSON-конверт понесёт вместе с ответом. В тексте они печатаются сразу, своим узлом,
    /// и здесь не копятся. Их отмечает и граница команды, у которой ведущий только по
    /// ссылке, поэтому список — за `RefCell`. Предупреждение с кодом лежит в том же списке:
    /// `warnings` и `warning_details` конверта собираются из него одним проходом.
    leading_warnings: RefCell<Vec<Warning>>,
}

impl Presenter {
    pub fn new(format: String, color_mode: ColorMode) -> Self {
        let no_color = matches!(color_mode, ColorMode::Disabled);
        Self {
            format,
            text: TextPresenter { no_color },
            json: JsonPresenter,
            leading_warnings: RefCell::default(),
        }
    }

    pub fn is_json(&self) -> bool {
        self.format == "json"
    }

    /// Предупреждения до ответа команды — с чем загрузился конфиг или что замок базы не
    /// взят: в тексте — узел `▲ <subject>` перед лентой команды, в JSON — хвост
    /// `warnings` любого конверта, который будет напечатан после. Пустой список ничего не
    /// печатает и ничего не запоминает.
    pub fn note_leading_warnings(&self, subject: &str, warnings: &[String]) {
        let warnings: Vec<Warning> = warnings.iter().cloned().map(Warning::Plain).collect();
        self.note_leading(subject, warnings);
    }

    /// То же для предупреждения границы команды: у предупреждения с кодом JSON-конверт
    /// понесёт и его `warning_details`; текст печатается так же, как у прочих.
    pub fn note_leading_warning(&self, subject: &str, warning: &Warning) {
        self.note_leading(subject, vec![warning.clone()]);
    }

    fn note_leading(&self, subject: &str, warnings: Vec<Warning>) {
        if warnings.is_empty() {
            return;
        }
        if self.is_json() {
            self.leading_warnings.borrow_mut().extend(warnings);
            return;
        }
        let details = warnings
            .iter()
            .map(|warning| format!("[warning] {}", warning.message()))
            .collect::<Vec<_>>()
            .join("\n");
        let node =
            TimelineItem::new(TimelineStatus::Succeeded, subject.to_owned()).with_detail(details);
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
        let leading = self.leading_warnings.borrow();
        if leading.is_empty() {
            self.json.print(envelope);
            return;
        }
        // Ведущие предупреждения идут после предупреждений команды: команда говорит о
        // своём первой, а `config init` ждёт своё предупреждение первым. Конверт
        // собирается заново с теми же полями, чтобы порядок ключей не менялся.
        let mut merged = Envelope {
            ok: envelope.ok,
            command: envelope.command.clone(),
            duration_ms: envelope.duration_ms,
            data: &envelope.data,
            warnings: envelope.warnings.clone(),
            warning_details: envelope.warning_details.clone(),
            steps: envelope.steps.clone(),
            error: envelope.error.clone(),
        };
        merged.follow_with(leading.iter());
        self.json.print(&merged);
    }
}

use crate::command_envelope::Envelope;
use crate::output::text::{JsonPresenter, TextPresenter, TimelineItem};
use serde::Serialize;

pub enum ColorMode {
    Enabled,
    Disabled,
}

pub struct Presenter {
    format: String,
    text: TextPresenter,
    json: JsonPresenter,
}

impl Presenter {
    pub fn new(format: String, color_mode: ColorMode) -> Self {
        let no_color = matches!(color_mode, ColorMode::Disabled);
        Self {
            format,
            text: TextPresenter { no_color },
            json: JsonPresenter,
        }
    }

    pub fn is_json(&self) -> bool {
        self.format == "json"
    }

    pub fn print_error(&self, msg: &str) {
        if self.is_json() {
            let env =
                Envelope::<serde_json::Value>::err("error", 0, serde_json::json!({"message": msg}));
            self.json.print(&env);
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
        if self.is_json() {
            self.json.print(envelope);
        }
        // text mode: callers render explicit timeline items.
    }
}

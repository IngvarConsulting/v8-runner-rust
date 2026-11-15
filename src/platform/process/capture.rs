//! Вывод разового процесса, который раннер держит в памяти, и вопрос, который процесс
//! повторяет на закрытом вводе.
//!
//! Единственный владелец обоих: потоки чтения `wait_for_output` читают stdout и stderr
//! только через [`read_stream`]
//! (`INV.PLATFORM.PROCESS-OUTPUT-IS-HELD-WITHIN-A-BOUND`, `INV.PLATFORM.A-REPEATED-IBCMD-QUESTION-IS-AN-INFOBASE-USER-REFUSAL`).

use std::collections::VecDeque;
use std::io::Read;
use std::sync::{Arc, OnceLock};

/// Сколько первых байт потока раннер держит в памяти.
const RETAINED_HEAD: usize = 56 << 20;
/// Сколько последних байт потока раннер держит в памяти: ответ утилиты — например, токен
/// поколения — пишется последним.
const RETAINED_TAIL: usize = 8 << 20;
/// Насколько далеко от среза ищется конец строки, чтобы резать по строкам.
const LINE_SEARCH: usize = 64 << 10;
/// Размер одного чтения из трубы.
const READ_CHUNK: usize = 64 << 10;

/// Сколько вывода нужно, чтобы назвать его повторённым вопросом.
const QUESTION_EVIDENCE: usize = 256 << 10;
/// Сколько вывода может предшествовать первому вопросу: строка-пояснение перед ним.
const QUESTION_PREAMBLE: usize = 4 << 10;
/// Самый длинный вопрос вместе с пояснением, повторяемым при каждом вопросе.
const QUESTION_MAX_UNIT: usize = 512;

/// Вопрос, который поток повторил, — его байты, как процесс их написал. Ставит поток чтения,
/// читает цикл ожидания процесса.
pub(super) type QuestionSlot = Arc<OnceLock<Vec<u8>>>;

/// Поток процесса в памяти раннера: начало, конец и сколько байт между ними опущено.
#[derive(Debug, Default)]
pub(super) struct CapturedStream {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    omitted: u64,
    /// Последний опущенный байт: конец ли строки стоит прямо перед хвостом.
    last_omitted: Option<u8>,
}

impl CapturedStream {
    fn push(&mut self, mut chunk: &[u8]) {
        if self.head.len() < RETAINED_HEAD {
            let taken = chunk.len().min(RETAINED_HEAD - self.head.len());
            self.head.extend_from_slice(&chunk[..taken]);
            chunk = &chunk[taken..];
        }
        if chunk.len() >= RETAINED_TAIL {
            let cut = chunk.len() - RETAINED_TAIL;
            let last = if cut > 0 {
                Some(chunk[cut - 1])
            } else {
                self.tail.back().copied()
            };
            if let Some(last) = last {
                self.last_omitted = Some(last);
            }
            self.omitted += (self.tail.len() + cut) as u64;
            self.tail.clear();
            self.tail.extend(&chunk[cut..]);
            return;
        }
        let overflow = (self.tail.len() + chunk.len()).saturating_sub(RETAINED_TAIL);
        if overflow > 0 {
            self.last_omitted = self.tail.get(overflow - 1).copied();
            self.tail.drain(..overflow);
            self.omitted += overflow as u64;
        }
        self.tail.extend(chunk);
    }

    /// Сколько байт потока раннер держит.
    #[cfg(test)]
    fn retained(&self) -> usize {
        self.head.len() + self.tail.len()
    }

    pub(super) fn total(&self) -> u64 {
        (self.head.len() + self.tail.len()) as u64 + self.omitted
    }

    /// Начало и конец потока, срезанные по строкам, если строка кончается рядом со срезом:
    /// пароль, который маскируют по строкам после захвата, срез не рвёт, и символ UTF-8 тоже.
    pub(super) fn finish(self) -> CapturedOutput {
        let mut head = self.head;
        let mut tail = Vec::from(self.tail);
        if self.omitted == 0 {
            head.append(&mut tail);
            return CapturedOutput {
                head,
                omitted: None,
                tail,
            };
        }
        let mut omitted = self.omitted;
        let search = head.len().saturating_sub(LINE_SEARCH);
        if let Some(end) = head[search..].iter().rposition(|byte| *byte == b'\n') {
            let keep = search + end + 1;
            omitted += (head.len() - keep) as u64;
            head.truncate(keep);
        }
        if self.last_omitted != Some(b'\n') {
            let search = tail.len().min(LINE_SEARCH);
            if let Some(start) = tail[..search].iter().position(|byte| *byte == b'\n') {
                omitted += (start + 1) as u64;
                tail.drain(..=start);
            }
        }
        // Строки длиннее поиска: срез двигается к границе символа UTF-8, если без него текст —
        // UTF-8. Вывод в кодовой странице OEM так и остаётся.
        let end = match std::str::from_utf8(&head) {
            Err(error) if error.error_len().is_none() => error.valid_up_to(),
            _ => head.len(),
        };
        omitted += (head.len() - end) as u64;
        head.truncate(end);
        if std::str::from_utf8(&tail).is_err() {
            if let Some(skip) = (1..=3).find(|skip| {
                tail.get(*skip..)
                    .is_some_and(|rest| std::str::from_utf8(rest).is_ok())
            }) {
                omitted += skip as u64;
                tail.drain(..skip);
            }
        }
        CapturedOutput {
            head,
            omitted: Some(omitted),
            tail,
        }
    }
}

/// Поток, каким его получает ответ: начало, сколько опущено (`None` — ничего, и всё лежит в
/// начале) и конец.
#[derive(Debug)]
pub(super) struct CapturedOutput {
    head: Vec<u8>,
    omitted: Option<u64>,
    tail: Vec<u8>,
}

impl CapturedOutput {
    /// Байты потока для журнала: начало, строка о пропуске и конец.
    pub(super) fn bytes(&self) -> Vec<u8> {
        let Some(omitted) = self.omitted else {
            return self.head.clone();
        };
        let line = omission_line(&self.head, omitted);
        [self.head.as_slice(), line.as_bytes(), self.tail.as_slice()].concat()
    }

    /// Текст потока: начало и конец читаются порознь, чтобы срез не перевёл их кодовую
    /// страницу ([`super::decode_output`]).
    pub(super) fn decode(self) -> String {
        let Some(omitted) = self.omitted else {
            return super::decode_output(self.head);
        };
        let line = omission_line(&self.head, omitted);
        let mut text = super::decode_output(self.head);
        text.push_str(&line);
        text.push_str(&super::decode_output(self.tail));
        text
    }
}

/// Отдельная строка о пропуске: ни один разборщик вывода её не примет за свою строку, и
/// разбор, закрытый от неизвестного, откажет, а не прочтёт половину.
fn omission_line(head: &[u8], omitted: u64) -> String {
    let lead = if head.is_empty() || head.ends_with(b"\n") {
        ""
    } else {
        "\n"
    };
    format!("{lead}[v8-runner: {omitted} bytes of output omitted]\n")
}

/// Читает поток процесса до конца, держа в памяти не больше [`RETAINED_HEAD`] +
/// [`RETAINED_TAIL`]. С `question` поток, как только наберёт [`QUESTION_EVIDENCE`], один раз
/// проверяет, не повторяет ли процесс с самого начала один и тот же вопрос; повторённый
/// вопрос кладётся в `question`, а чтение идёт дальше до конца потока.
pub(super) fn read_stream(
    mut stream: impl Read,
    question: Option<&QuestionSlot>,
) -> CapturedStream {
    let mut captured = CapturedStream::default();
    let mut buffer = vec![0u8; READ_CHUNK];
    let mut asked = question.is_none();
    loop {
        let read = match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        captured.push(&buffer[..read]);
        if !asked && captured.total() >= QUESTION_EVIDENCE as u64 {
            asked = true;
            if let (Some(slot), Some(unit)) = (question, repeated_question(&captured.head)) {
                let _ = slot.set(unit.to_vec());
            }
        }
    }
    captured
}

/// Вопрос, который процесс повторяет с самого начала вывода: всё после пояснения не длиннее
/// [`QUESTION_PREAMBLE`] — один и тот же кусок не длиннее [`QUESTION_MAX_UNIT`], и в нём есть
/// не только пробелы. Признак структурный: ни язык, ни кодировка вопроса его не меняют.
/// Процесс, который сделал хоть что-то, кроме вопроса, вывел бы больше пояснения.
pub(super) fn repeated_question(stream: &[u8]) -> Option<&[u8]> {
    let length = stream.len();
    if length < QUESTION_EVIDENCE {
        return None;
    }
    // Наименьший шаг: всякий другой шаг такого длинного повтора кратен ему, так что кусок из
    // одних пробелов у наименьшего шага значит пробелы и у всех.
    let (period, start) = (1..=QUESTION_MAX_UNIT).find_map(|period| {
        let start = periodic_run_start(stream, period);
        (start <= QUESTION_PREAMBLE).then_some((period, start))
    })?;
    // Кусок начинается после последнего конца строки перед ним: пояснение, кончающееся теми же
    // байтами, что и вопрос, не повернёт улику.
    let start = stream[start..start + period]
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(start, |line_end| start + line_end + 1);
    let unit = &stream[start..start + period];
    unit.iter()
        .any(|byte| !byte.is_ascii_whitespace())
        .then_some(unit)
}

/// Начало хвоста `stream`, который повторяет себя с шагом `period`. Неверный шаг
/// останавливает обход на первом же несовпадении.
fn periodic_run_start(stream: &[u8], period: usize) -> usize {
    let mut index = stream.len();
    while index > period && stream[index - 1] == stream[index - 1 - period] {
        index -= 1;
    }
    index.saturating_sub(period)
}

#[cfg(test)]
mod tests {
    use super::*;

    const UTF8_QUESTION: &str = "Имя пользователя: ";
    /// «Имя пользователя: » в кодовой странице OEM 866: так пишет `ibcmd` 8.3 на Windows.
    const CP866_QUESTION: &[u8] = &[
        0x88, 0xAC, 0xEF, 0x20, 0xAF, 0xAE, 0xAB, 0xEC, 0xA7, 0xAE, 0xA2, 0xA0, 0xE2, 0xA5, 0xAB,
        0xEF, 0x3A, 0x20,
    ];
    const PREAMBLE: &str =
        "Для выполнения операции требуется аутентификация в информационной базе\n";

    fn asked(preamble: &[u8], unit: &[u8], bytes: usize) -> Vec<u8> {
        let mut stream = preamble.to_vec();
        while stream.len() < bytes {
            stream.extend_from_slice(unit);
        }
        stream
    }

    fn read_all(bytes: &[u8], question: Option<&QuestionSlot>) -> CapturedStream {
        read_stream(std::io::Cursor::new(bytes.to_vec()), question)
    }

    /// Поток в сто мегабайт держится в пределе: начало и конец целы, середина названа строкой
    /// о пропуске (`INV.PLATFORM.PROCESS-OUTPUT-IS-HELD-WITHIN-A-BOUND`).
    #[test]
    fn a_flood_of_output_is_kept_within_the_bound() {
        let mut captured = CapturedStream::default();
        let line = b"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde\n";
        captured.push(b"first line\n");
        let mut total = 11u64;
        while total < 100 << 20 {
            captured.push(line);
            total += line.len() as u64;
            assert!(
                captured.retained() <= RETAINED_HEAD + RETAINED_TAIL,
                "{}",
                captured.retained()
            );
        }
        captured.push(b"last line\n");
        total += 10;
        assert_eq!(captured.total(), total);

        let text = captured.finish().decode();
        assert!(text.starts_with("first line\n"), "{}", &text[..64]);
        assert!(text.ends_with(&format!(
            "{}last line\n",
            std::str::from_utf8(line).unwrap()
        )));
        let omission = text
            .lines()
            .find(|line| line.starts_with("[v8-runner: "))
            .expect("the omission is named");
        assert!(
            omission.ends_with(" bytes of output omitted]"),
            "{omission}"
        );
        assert!(text.len() <= RETAINED_HEAD + RETAINED_TAIL + omission.len() + 2);
        assert!(
            text.lines().all(|kept| kept == "first line"
                || kept == "last line"
                || kept == omission
                || kept.as_bytes() == &line[..line.len() - 1]),
            "a cut tore a line"
        );
    }

    /// Один кусок больше предела тоже не держится целиком.
    #[test]
    fn one_huge_chunk_is_kept_within_the_bound() {
        let mut captured = CapturedStream::default();
        captured.push(&vec![b'x'; 80 << 20]);
        assert_eq!(captured.retained(), RETAINED_HEAD + RETAINED_TAIL);
        assert_eq!(captured.total(), 80 << 20);
    }

    /// Вывод в пределе возвращается как есть, без строки о пропуске.
    #[test]
    fn output_within_the_bound_is_kept_as_written() {
        let captured = read_all("строка\nответ\n".as_bytes(), None);
        assert_eq!(captured.finish().decode(), "строка\nответ\n");
        let captured = read_all(b"bytes", None);
        assert_eq!(captured.finish().bytes(), b"bytes");
    }

    /// Срез не рвёт символ UTF-8: без концов строк рядом срез падает на середину символа, а
    /// текст всё равно читается как UTF-8, а не в кодовой странице OEM.
    #[test]
    fn a_cut_inside_a_character_keeps_both_parts_readable() {
        let mut captured = CapturedStream::default();
        captured.push(b"a");
        let letters = "ж".repeat(40 << 20);
        captured.push(letters.as_bytes());
        let text = captured.finish().decode();
        assert!(text.starts_with("aж"), "{}", &text[..8]);
        assert!(text.ends_with("жж"));
    }

    /// Байты вопроса в OEM 866 — те, что пишет платформа, а не выдумка теста.
    #[cfg(windows)]
    #[test]
    fn the_oem_question_is_the_platform_text() {
        assert_eq!(
            super::super::decode_in_code_page(CP866_QUESTION, 866),
            UTF8_QUESTION
        );
    }

    /// Вопрос, повторённый с начала вывода, узнаётся в любой кодировке и с пояснением перед
    /// ним; свидетельство — сам вопрос.
    #[test]
    fn a_question_repeated_from_the_start_is_recognised_in_any_encoding() {
        let utf8 = asked(PREAMBLE.as_bytes(), UTF8_QUESTION.as_bytes(), 300 << 10);
        assert_eq!(repeated_question(&utf8), Some(UTF8_QUESTION.as_bytes()));

        let oem = asked(
            b"\x8f\xae\xa4\xe1\xaa\xa0\xa7\xaa\xa0\r\n",
            CP866_QUESTION,
            300 << 10,
        );
        assert_eq!(repeated_question(&oem), Some(CP866_QUESTION));

        let both = format!("{PREAMBLE}{UTF8_QUESTION}");
        let paired = asked(b"", both.as_bytes(), 300 << 10);
        let unit = repeated_question(&paired).expect("the pair repeats");
        assert_eq!(unit.len(), both.len());

        let slot = QuestionSlot::default();
        let captured = read_all(&utf8, Some(&slot));
        assert_eq!(
            slot.get().map(Vec::as_slice),
            Some(UTF8_QUESTION.as_bytes())
        );
        assert_eq!(captured.total(), utf8.len() as u64);
    }

    /// Обычный вывод вопросом не считается: разные строки, длинный вывод после работы с
    /// повторами в конце, пустые строки, вопрос, заданный один раз, и короткий поток.
    #[test]
    fn ordinary_output_is_not_a_question() {
        let mut varied = Vec::new();
        let mut index = 0;
        while varied.len() < 300 << 10 {
            varied
                .extend_from_slice(format!("Загружен объект Справочник.Номер{index}\n").as_bytes());
            index += 1;
        }
        assert_eq!(repeated_question(&varied), None);

        let mut worked_then_repeated = varied[..64 << 10].to_vec();
        worked_then_repeated = asked(
            &worked_then_repeated,
            b"WARNING: no transaction\n",
            400 << 10,
        );
        assert_eq!(repeated_question(&worked_then_repeated), None);

        let blank = vec![b'\n'; 300 << 10];
        assert_eq!(repeated_question(&blank), None);
        let spaces = asked(b"", b" \r\n\t", 300 << 10);
        assert_eq!(repeated_question(&spaces), None);

        let once = format!("Введите пароль: \n{}\n", "1".repeat(40));
        assert_eq!(repeated_question(once.as_bytes()), None);

        let short = asked(b"", UTF8_QUESTION.as_bytes(), 200 << 10);
        assert_eq!(repeated_question(&short), None);

        let slot = QuestionSlot::default();
        let _ = read_all(&varied, Some(&slot));
        assert!(slot.get().is_none());
    }

    /// Без `question` поток вопроса не ищет.
    #[test]
    fn a_stream_without_a_slot_is_not_examined() {
        let utf8 = asked(b"", UTF8_QUESTION.as_bytes(), 300 << 10);
        let captured = read_all(&utf8, None);
        assert_eq!(captured.total(), utf8.len() as u64);
    }
}

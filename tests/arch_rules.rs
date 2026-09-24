//! Гейт правил продукта: `spec/rules/`.
//!
//! Проверяется не форма прозы, а то, без чего запись перестаёт быть обязательством:
//! у неё есть имя, это имя одно на весь реестр, названные проверки существуют, а набор
//! полей закрыт. Всё остальное — текст правила — держит разбор, а не гейт.
//!
//! Обход **рекурсивный**: записи лежат по областям продукта, и плоское чтение каталога
//! нашло бы ноль записей и прошло зелёным. Поэтому здесь же стоит пол по числу записей:
//! страж, который нечего проверять, — не страж.

mod guardrail_support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use syn::visit::Visit;

/// Пол заведомо ниже числа правил. Он ловит не убыль обязательств, а поломку обхода:
/// неверный корень или нерекурсивное чтение дают ноль.
const AT_LEAST: usize = 140;

/// Поля записи закрыты. `id` и `check` обязательны; `gap` называет задачу, пока
/// обязательство не выполнено; `version` и `artifact` есть у записи, закрепляющей форму
/// данных. Ничего сверх — иначе прежние `status`, `governs` и `decision` вернутся по
/// одному за раз.
const ALLOWED: &[&str] = &["id", "check", "gap", "version", "artifact"];

/// Поля, обещанные README одним значением. Список читатели берут через `.first()`, и
/// `id: [A, B]` прошло бы молча, назвав два имени там, где обещано одно.
const SCALAR: &[&str] = &["id", "gap", "version", "artifact"];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn rules_root() -> PathBuf {
    repo_root().join("spec/rules")
}

/// Имя записи для отчётов и перечней — всегда через косую черту.
///
/// `Path::display()` на Windows дал бы `cli\\text-output.md`, и `NORMATIVE_NUMERALS`
/// разошёлся бы со своим же перечнем на каждой строке.
fn shown(path: &Path) -> String {
    path.strip_prefix(rules_root())
        .unwrap_or(path)
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Все записи правил, в устойчивом порядке. README областей записью не является.
fn rule_paths(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let entries = std::fs::read_dir(dir).expect("rules directory is readable");
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(rule_paths(&path));
        } else if path.extension().is_some_and(|value| value == "md")
            && path.file_name().is_some_and(|value| value != "README.md")
        {
            found.push(path);
        }
    }
    found.sort();
    found
}

/// Поля записи: скаляр или плоский список. Больше записи не нужно, и большего здесь
/// намеренно не читают.
fn front_matter(text: &str) -> Result<BTreeMap<String, Vec<String>>, String> {
    let rest = text
        .strip_prefix("---\n")
        .ok_or_else(|| "no front matter".to_owned())?;
    let end = rest
        .find("\n---\n")
        .ok_or_else(|| "front matter is not closed".to_owned())?;
    let mut props: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut key = String::new();
    for line in rest[..end].lines() {
        if let Some(item) = line.strip_prefix("  - ") {
            if key.is_empty() {
                return Err(format!("list item before any key: {line}"));
            }
            props
                .entry(key.clone())
                .or_default()
                .push(item.trim().to_owned());
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(format!("line is neither a key nor a list item: {line}"));
        };
        key = name.trim().to_owned();
        let value = value.trim();
        let values = if value.is_empty() {
            Vec::new()
        } else if let Some(inner) = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
            inner
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_owned)
                .collect()
        } else {
            vec![value.to_owned()]
        };
        if props.insert(key.clone(), values).is_some() {
            return Err(format!("ключ `{key}` назван дважды"));
        }
    }
    Ok(props)
}

fn read_rules() -> Vec<(String, BTreeMap<String, Vec<String>>)> {
    rule_paths(&rules_root())
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).expect("rule is readable");
            let shown = shown(&path);
            let props = front_matter(&text).unwrap_or_else(|error| panic!("{shown}: {error}"));
            (shown, props)
        })
        .collect()
}

#[test]
fn the_registry_is_read_whole() {
    let rules = read_rules();
    assert!(
        rules.len() >= AT_LEAST,
        "прочитано {} записей при поле {AT_LEAST}: корень или обход сломаны",
        rules.len()
    );
}

/// Имя записи — единственный способ на неё сослаться, и оно обязано быть одно.
///
/// Прежде это держала пара «символ и путь пишут друг друга»; имя файла теперь называет
/// тему, а не символ, поэтому уникальность проверяется прямо.
#[test]
fn a_name_belongs_to_exactly_one_rule() {
    let mut seen: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (shown, props) in read_rules() {
        let Some(id) = props.get("id").and_then(|values| values.first()) else {
            panic!("{shown}: запись без поля `id`");
        };
        seen.entry(id.clone()).or_default().push(shown);
    }
    let clashes: Vec<String> = seen
        .iter()
        .filter(|(_, files)| files.len() > 1)
        .map(|(id, files)| format!("{id}: {}", files.join(", ")))
        .collect();

    assert!(
        clashes.is_empty(),
        "одно имя у нескольких записей:\n{}",
        clashes.join("\n")
    );
}

/// Набор полей закрыт: иначе прежняя схема возвращается по полю за раз.
#[test]
fn a_rule_carries_only_the_fields_the_readme_publishes() {
    let mut wrong = Vec::new();
    for (shown, props) in read_rules() {
        for key in props.keys() {
            if !ALLOWED.contains(&key.as_str()) {
                wrong.push(format!("{shown}: поле `{key}` схемой не предусмотрено"));
            }
        }
        if !props.contains_key("id") {
            wrong.push(format!("{shown}: нет поля `id`"));
        }
        if !props.contains_key("check") {
            wrong.push(format!("{shown}: нет поля `check`"));
        }
        for key in SCALAR {
            if let Some(values) = props.get(*key) {
                if values.len() != 1 {
                    wrong.push(format!(
                        "{shown}: поле `{key}` обещано одним значением, а несёт {}",
                        values.len()
                    ));
                }
            }
        }
    }

    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Функции файла, поделённые на тесты и прочие, — по синтаксическому дереву.
///
/// Разбор строк здесь был и проигрывал: фикстура сразу за телом теста, помощник первой
/// строкой внутри теста, атрибут в комментарии — каждый раз находилась новая форма,
/// которую он принимал за тест. Синтаксис отвечает точно, а разборщик у тестов уже есть —
/// `tests/guardrail_support.rs`. Вложенная функция видна тоже: она — не тест, пока у неё
/// нет своего атрибута.
#[derive(Default)]
struct Functions {
    tests: BTreeSet<String>,
    plain: BTreeSet<String>,
}

impl<'ast> Visit<'ast> for Functions {
    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        let name = item.sig.ident.to_string();
        let is_test = item.attrs.iter().any(|attr| {
            attr.path()
                .segments
                .last()
                .is_some_and(|segment| segment.ident == "test")
        });
        if is_test {
            self.tests.insert(name);
        } else {
            self.plain.insert(name);
        }
        syn::visit::visit_item_fn(self, item);
    }
}

fn functions_of(source: &Path) -> Functions {
    let mut functions = Functions::default();
    functions.visit_file(&guardrail_support::parse_rust_file(source));
    functions
}

/// Названная проверка существует. Гейт не судит, что она доказывает, — он не даёт
/// сослаться на то, чего нет.
#[test]
fn every_named_check_exists() {
    let root = repo_root();
    let mut unresolved = Vec::new();
    for (shown, props) in read_rules() {
        for address in props.get("check").into_iter().flatten() {
            let Some((file, name)) = address.split_once("::") else {
                unresolved.push(format!("{shown}: адрес без имени проверки: {address}"));
                continue;
            };
            let source = root.join(file);
            if !source.is_file() {
                unresolved.push(format!("{shown}: нет файла {file}"));
                continue;
            }
            // Совпадения имени мало: фикстура тоже объявлена `fn`, и ссылка на неё прошла
            // бы зелёной, ничего не доказывая. Проверка — функция с атрибутом теста.
            let functions = functions_of(&source);
            if functions.tests.contains(name) {
                continue;
            }
            if functions.plain.contains(name) {
                unresolved.push(format!(
                    "{shown}: {file}::{name} — не проверка, а обычная функция"
                ));
            } else {
                unresolved.push(format!("{shown}: в {file} нет проверки {name}"));
            }
        }
    }

    assert!(unresolved.is_empty(), "{}", unresolved.join("\n"));
}

/// Запись без проверок называет задачу: обязательство без свидетельства и без разрыва —
/// обещание, за которым никто не стоит.
#[test]
fn a_rule_without_checks_names_its_gap() {
    let mut wrong = Vec::new();
    for (shown, props) in read_rules() {
        let empty = props.get("check").is_none_or(|values| values.is_empty());
        let gap = props.get("gap").and_then(|values| values.first());
        match (empty, gap) {
            (true, None) => wrong.push(format!("{shown}: нет ни проверок, ни `gap`")),
            (false, Some(_)) => {
                wrong.push(format!("{shown}: `gap` стоит при заполненном `check`"));
            }
            _ => {}
        }
        if let Some(gap) = gap {
            if !gap.starts_with("https://github.com/") {
                wrong.push(format!("{shown}: `gap` не ведёт на задачу: {gap}"));
            }
        }
    }

    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Запись, закрепляющая форму данных, называет свой артефакт и номер формы.
///
/// Номер растёт, когда меняется сама форма; что он вырос, гейт не доказывает — это разбор.
/// Здесь держится меньшее и проверяемое: артефакт лежит на диске, номер — целое от единицы.
#[test]
fn a_pinned_form_names_an_artifact_that_exists() {
    let root = repo_root();
    let mut wrong = Vec::new();
    for (shown, props) in read_rules() {
        let artifact = props.get("artifact").and_then(|values| values.first());
        let version = props.get("version").and_then(|values| values.first());
        match (artifact, version) {
            (None, None) => continue,
            (Some(artifact), Some(version)) => {
                if !root.join(artifact).is_file() {
                    wrong.push(format!("{shown}: артефакта нет на диске: {artifact}"));
                }
                if version.parse::<u32>().unwrap_or(0) == 0 {
                    wrong.push(format!(
                        "{shown}: номер формы не целое от единицы: {version}"
                    ));
                }
            }
            (Some(_), None) => wrong.push(format!("{shown}: артефакт без номера формы")),
            (None, Some(_)) => wrong.push(format!("{shown}: номер формы без артефакта")),
        }
    }

    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Язык и тело каждого блока кода в разделе «Пример».
///
/// Блоков бывает несколько: запись показывает и обычный ответ, и ответ превью.
/// Проверяется каждый — иначе второй пример держался бы на внимательности автора.
fn example_blocks(text: &str) -> Vec<(String, String)> {
    let Some(heading) = text.find("\n## Пример\n") else {
        return Vec::new();
    };
    let section = &text[heading + "\n## Пример\n".len()..];
    // Раздел кончается следующим заголовком того же уровня — но заголовком, а не строкой
    // внутри ограды: пример умеет показывать и разметку записи.
    let mut end = section.len();
    let mut offset = 0;
    for (line, fenced) in lines_outside_fences(section) {
        if !fenced && line.starts_with("## ") {
            end = offset;
            break;
        }
        offset += line.len() + 1;
    }
    let section = &section[..end.min(section.len())];

    let mut blocks = Vec::new();
    let mut rest = section;
    while let Some(open) = rest.find("```") {
        let after_open = &rest[open + 3..];
        let Some(newline) = after_open.find('\n') else {
            break;
        };
        let language = after_open[..newline].trim().to_owned();
        let body = &after_open[newline + 1..];
        let Some(close) = body.find("\n```") else {
            break;
        };
        blocks.push((language, body[..close + 1].to_owned()));
        rest = &body[close + "\n```".len()..];
    }
    blocks
}

fn parse_example(
    language: &str,
    example: &str,
    name: &str,
    wrong: &mut Vec<String>,
) -> Option<serde_json::Value> {
    let parsed = match language {
        "yaml" => serde_yaml::from_str::<serde_json::Value>(example).map_err(|e| e.to_string()),
        _ => serde_json::from_str::<serde_json::Value>(example).map_err(|e| e.to_string()),
    };
    match parsed {
        Ok(value) => Some(value),
        Err(error) => {
            wrong.push(format!("{name}: example is not valid {language}: {error}"));
            None
        }
    }
}

/// Проверяет, что `fragment` целиком встречается в `whole`.
///
/// У объекта сверяются только названные фрагментом ключи, у всего остального —
/// равенство. Так пример показывает одну запись закреплённого документа, не переписывая
/// документ целиком.
fn contains(whole: &serde_json::Value, fragment: &serde_json::Value) -> bool {
    match (whole, fragment) {
        (serde_json::Value::Object(whole), serde_json::Value::Object(fragment)) => fragment
            .iter()
            .all(|(key, value)| whole.get(key).is_some_and(|found| contains(found, value))),
        _ => whole == fragment,
    }
}

/// Раздел «Пример» у правила, закрепляющего форму, — не проза, а проверяемый экземпляр.
///
/// Если артефакт — схема, пример обязан её пройти; если артефакт не схема, а
/// закреплённый документ, пример обязан быть его фрагментом. Иначе пример живёт своей
/// жизнью и через два изменения формы врёт читателю ровно там, где тот ему верит.
#[test]
fn every_pinned_example_passes_its_own_form() {
    let root = repo_root();
    let mut wrong = Vec::new();

    for path in rule_paths(&rules_root()) {
        let text = std::fs::read_to_string(&path).expect("rule is readable");
        let name = shown(&path);
        let props = front_matter(&text).unwrap_or_else(|error| panic!("{name}: {error}"));
        let Some(artifact) = props.get("artifact").and_then(|values| values.first()) else {
            continue;
        };

        let examples = example_blocks(&text);
        if examples.is_empty() {
            wrong.push(format!("{name}: no example block"));
            continue;
        }

        let artifact_text = match std::fs::read_to_string(root.join(artifact)) {
            Ok(text) => text,
            Err(error) => {
                wrong.push(format!(
                    "{name}: artifact {artifact} is unreadable: {error}"
                ));
                continue;
            }
        };
        let artifact_value: serde_json::Value = match serde_json::from_str(&artifact_text) {
            Ok(value) => value,
            Err(error) => {
                wrong.push(format!("{name}: artifact {artifact} is not json: {error}"));
                continue;
            }
        };

        for (language, example) in &examples {
            if let Some(line_kinds) = artifact_value
                .get("line_kinds")
                .and_then(serde_json::Value::as_object)
            {
                // Артефакт-грамматика описывает не документ, а строки. Пример к нему —
                // кусок настоящего вывода, и проверяется он построчно.
                let mut patterns = Vec::new();
                for pattern in line_kinds
                    .values()
                    .filter_map(|kind| kind.get("pattern").and_then(serde_json::Value::as_str))
                {
                    match regex::Regex::new(pattern) {
                        Ok(compiled) => patterns.push(compiled),
                        Err(error) => wrong.push(format!(
                            "{name}: образец строки в {artifact} не компилируется: {error}"
                        )),
                    }
                }
                for line in example.lines().filter(|line| !line.trim().is_empty()) {
                    if !patterns.iter().any(|pattern| pattern.is_match(line)) {
                        wrong.push(format!(
                            "{name}: example line matches no kind of {artifact}: {line:?}"
                        ));
                    }
                }
            } else if artifact_value.get("$schema").is_some() {
                let Some(parsed) = parse_example(language, example, &name, &mut wrong) else {
                    continue;
                };
                let validator = match jsonschema::validator_for(&artifact_value) {
                    Ok(validator) => validator,
                    Err(error) => {
                        wrong.push(format!(
                            "{name}: artifact {artifact} is not a schema: {error}"
                        ));
                        continue;
                    }
                };
                let errors: Vec<String> = validator
                    .iter_errors(&parsed)
                    .map(|error| format!("{} at {}", error, error.instance_path))
                    .collect();
                if !errors.is_empty() {
                    wrong.push(format!(
                        "{name}: example fails its own form {artifact}:\n{}",
                        errors.join("\n")
                    ));
                }
            } else {
                let Some(parsed) = parse_example(language, example, &name, &mut wrong) else {
                    continue;
                };
                if !contains(&artifact_value, &parsed) {
                    wrong.push(format!(
                        "{name}: example is not a fragment of the pinned {artifact}"
                    ));
                }
            }
        }
    }

    assert!(
        wrong.is_empty(),
        "pinned examples are prose, not pinned form:\n{}",
        wrong.join("\n")
    );
}

/// Счётные слова, которые перечень ловит в нормативной части.
///
/// «Семью» здесь нет намеренно: в реестре это винительный падеж «семьи» («в семью»), а не
/// творительный числа, и слово стоит ровно в том значении. Цифрами записанное число
/// перечень тоже не видит — отделить обещание от номера версии, кода выхода и пути прозой
/// нельзя.
const COUNTING_WORDS: &[&str] = &[
    "два",
    "две",
    "двух",
    "двумя",
    "двое",
    "оба",
    "обе",
    "обоих",
    "обеих",
    "обоими",
    "обеим",
    "три",
    "трёх",
    "тремя",
    "трое",
    "четыре",
    "четырёх",
    "четырьмя",
    "пять",
    "пяти",
    "шесть",
    "шести",
    "семь",
    "семи",
    "восемь",
    "восьми",
    "пятью",
    "шестью",
    "восемью",
    "девятью",
    "десятью",
    "четверо",
    "пятеро",
    "двоих",
    "девять",
    "девяти",
    "десять",
    "десяти",
    "одиннадцать",
    "двенадцать",
    "тринадцать",
    "четырнадцать",
    "пятнадцать",
    "шестнадцать",
    "семнадцать",
    "восемнадцать",
    "девятнадцать",
    "двадцать",
    "двадцати",
    "тридцать",
    "тридцати",
    "сорок",
    "сорока",
    "пятьдесят",
    "сто",
    "вдвое",
    "втрое",
    "дважды",
    "трижды",
];

/// Строки записи вместе с признаком «внутри огороженного блока».
///
/// Огороженный пример — часть прозы, но заголовком его строка быть не может: пример
/// умеет показывать и разметку записи, и тогда `## ` внутри него оборвал бы разбор
/// раньше времени.
fn lines_outside_fences(text: &str) -> impl Iterator<Item = (&str, bool)> {
    let mut fenced = false;
    text.lines().map(move |line| {
        if line.trim_start().starts_with("```") {
            // Сама ограда — не проза и не заголовок ни с какой стороны.
            fenced = !fenced;
            return (line, true);
        }
        (line, fenced)
    })
}

/// Нормативная часть правила — та, где число значит «столько сейчас»: тело до первого
/// подраздела. Ниже идут примеры и оговорки, верные для своего дня.
fn normative_prose(text: &str) -> String {
    let body = match text.split_once("\n---\n") {
        Some((_, rest)) => rest,
        None => text,
    };
    let mut kept = Vec::new();
    for (line, fenced) in lines_outside_fences(body) {
        if !fenced && line.starts_with("## ") {
            break;
        }
        kept.push(line);
    }
    kept.join("\n")
}

/// Счётные слова нормативной части, все до одного и в порядке появления.
///
/// Повторы не схлопываются: у записи бывает несколько счётов одним словом, и схлопнутое
/// множество не заметило бы ни нового, ни пропавшего.
fn counting_words_in(prose: &str) -> Vec<String> {
    prose
        .split(|ch: char| !ch.is_alphabetic())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .filter(|word| COUNTING_WORDS.contains(&word.as_str()))
        .collect()
}

fn normative_numerals() -> Vec<(String, String)> {
    let mut rows = Vec::new();
    for path in rule_paths(&rules_root()) {
        let text = std::fs::read_to_string(&path).expect("rule is readable");
        let words = counting_words_in(&normative_prose(&text));
        if words.is_empty() {
            continue;
        }
        rows.push((shown(&path), words.join(",")));
    }
    rows.sort();
    rows
}

/// Числительные нормативной части — по правилу, где они стоят.
///
/// Число в теле правила значит «столько сейчас», то есть это обещание. Обещание держит
/// проверка; перечень требует лишь, чтобы новое обещание назвали — гейт не решает,
/// верно ли число, он не даёт появиться неназванному.
///
/// Перечнем, а не разбором: русское числительное прозой не разбирается надёжно, и
/// догадываться, что оно считает, гейт не должен. Тот же приём держит состав листьев
/// (`src/cli/global_flags_expected.in`) и состав инструментов MCP, а ближайший образец —
/// `tests/tool_output_contract.rs::PROSE_DEBT`. От него перечень отличается тем, что
/// расти ему можно: новое правило с числом — обычное дело, а долг по прозе только
/// сокращают.
const NORMATIVE_NUMERALS: &[(&str, &str)] = &[
    ("cli/a-refusal-names-the-missing-credential-level.md", "три"),
    ("cli/apply-is-a-separate-step.md", "два,оба,три"),
    ("cli/concurrent-processes-are-serialized.md", "два"),
    ("cli/init-declares-and-clone-pulls.md", "двух"),
    ("cli/text-output.md", "двух,дважды"),
    ("cli/via-is-rejected-where-there-is-no-choice.md", "обеих"),
    (
        "config/a-providers-key-is-named-after-its-command.md",
        "тринадцать",
    ),
    (
        "config/a-standalone-target-accepts-either-gate-key.md",
        "двух,оба",
    ),
    ("config/target-declarations-are-exclusive.md", "три"),
    ("config/v8project-schema.md", "двумя,обеих,два"),
    ("mcp/admission-is-shared-by-both-transports.md", "обоих"),
    ("mcp/published-tool-surface.md", "восемь,три"),
    ("mcp/surface-stays-explicit.md", "оба"),
    ("platform/agent-session-lives-with-the-lock.md", "два,два"),
    ("platform/edt-has-two-execution-modes.md", "два"),
    ("platform/prose-debt-only-shrinks.md", "трёх"),
    (
        "use-cases/a-generation-token-is-compared-within-its-own-tool.md",
        "сорок,сорок,двух",
    ),
    (
        "use-cases/a-push-into-a-base-that-moved-ahead-is-refused.md",
        "оба",
    ),
    ("use-cases/edt-keeps-two-change-contexts.md", "две"),
    ("use-cases/replacing-a-user-directory-asks-first.md", "три"),
    ("wire/check-data.md", "двумя,три"),
    ("wire/clone-data.md", "два,обоими,четырёх"),
    ("wire/command-envelope.md", "два,четыре,обоими,два,два"),
    ("wire/infobase-restore-data.md", "двумя"),
    ("wire/launch-data.md", "двух"),
    ("wire/load-data.md", "тремя"),
];

/// Новое число в нормативной части обязано быть названо здесь.
///
/// Гейт не судит, верно ли число: он не даёт ему появиться молча. Назвавший строку автор
/// отвечает на вопрос, обещание это или замер, — и либо заводит проверку, либо
/// переписывает предложение замером с датой.
///
/// Чего перечень не видит: предложение переписали, слово осталось прежним, а считает оно
/// теперь другое — «четыре пути» стали «четырьмя шагами». Закрыть это значило бы решать,
/// что именно числительное считает, а прозой это не разбирается.
#[test]
fn every_number_in_a_normative_block_is_named_here() {
    let actual = normative_numerals();
    let expected: Vec<(String, String)> = NORMATIVE_NUMERALS
        .iter()
        .map(|(record, words)| ((*record).to_owned(), (*words).to_owned()))
        .collect();
    if actual == expected {
        return;
    }

    // Печатается расхождение и готовая строка, а не два перечня по два десятка записей:
    // иначе сообщение нечитаемо ровно тогда, когда его читают.
    let mut differs = Vec::new();
    for (record, words) in &actual {
        match expected.iter().find(|(named, _)| named == record) {
            None => differs.push(format!("+ (\"{record}\", \"{words}\"),")),
            Some((_, named)) if named != words => {
                differs.push(format!("~ (\"{record}\", \"{words}\"), было \"{named}\""));
            }
            Some(_) => {}
        }
    }
    for (record, _) in &expected {
        if !actual.iter().any(|(found, _)| found == record) {
            differs.push(format!("- (\"{record}\", …),"));
        }
    }
    if differs.is_empty() {
        // Состав тот же, а порядок другой: перечень отсортирован по пути правила внутри
        // `rules/`. Без этой ветки сообщение было бы пустым ровно на самой частой ошибке.
        panic!(
            "состав перечня NORMATIVE_NUMERALS верен, а порядок строк — нет: \
             сортировка по пути правила внутри `spec/rules/`"
        );
    }

    panic!(
        "числа нормативной части разошлись с перечнем NORMATIVE_NUMERALS:\n{}\n",
        differs.join("\n")
    );
}

/// Номер ADR не адресует ничего и не должен вернуться.
///
/// Прежний слой ADR был заменён реестром решений, а тот — этими правилами; оба сняты, и
/// оба каталога удалены вместе с таблицей судьбы, которая переводила номер во владельца.
/// Ссылка по номеру теперь ведёт в никуда, и перевести её обратно нечем. Правило ищут по
/// предмету или по имени проверки.
#[test]
fn old_adr_numbers_address_nothing() {
    let root = repo_root();
    let number = regex::Regex::new(r"ADR-\d{4}").expect("regex");
    let mut offenders = Vec::new();
    let mut pending: Vec<PathBuf> = [
        "src", "tests", "docs", "spec", "scripts", "SKILL", "examples", ".github",
    ]
    .iter()
    .map(|dir| root.join(dir))
    .filter(|dir| dir.exists())
    .collect();
    for entry in std::fs::read_dir(&root).expect("repo root is readable") {
        let path = entry.expect("directory entry").path();
        if path.extension().and_then(|value| value.to_str()) == Some("md") {
            pending.push(path);
        }
    }
    while let Some(path) = pending.pop() {
        if path.is_dir() {
            for entry in std::fs::read_dir(&path).expect("directory is readable") {
                pending.push(entry.expect("directory entry").path());
            }
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (index, line) in text.lines().enumerate() {
            if let Some(found) = number.find(line) {
                offenders.push(format!(
                    "{}:{}: {}",
                    path.strip_prefix(&root).unwrap_or(&path).display(),
                    index + 1,
                    found.as_str()
                ));
            }
        }
    }
    offenders.sort();
    assert!(
        offenders.is_empty(),
        "номер ADR не адресует ничего; назовите правило из spec/rules/:\n{}",
        offenders.join("\n")
    );
}

/// Область имени и каталог называют друг друга.
///
/// Прежде это держала пара «символ и путь пишут друг друга» целиком. Имя файла теперь
/// называет тему, а не символ, и от пары осталась одна половина: правило `rules/cli/…`
/// с именем `INV.WIRE.…` прошло бы молча и нашлось бы не там, где его ищут.
#[test]
fn an_area_and_its_directory_name_each_other() {
    let mut wrong = Vec::new();
    for path in rule_paths(&rules_root()) {
        let text = std::fs::read_to_string(&path).expect("rule is readable");
        let shown = shown(&path);
        let props = front_matter(&text).unwrap_or_else(|error| panic!("{shown}: {error}"));
        let Some(id) = props.get("id").and_then(|values| values.first()) else {
            continue;
        };
        let mut parts = id.split('.');
        let kind = parts.next().unwrap_or_default();
        if kind != "INV" && kind != "CTR" {
            wrong.push(format!(
                "{shown}: приставка имени не `INV` и не `CTR`: {id}"
            ));
        }
        let Some(area) = parts.next() else {
            wrong.push(format!("{shown}: имя без области: {id}"));
            continue;
        };
        let directory = path
            .parent()
            .and_then(|parent| parent.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_uppercase();
        if area != directory {
            wrong.push(format!(
                "{shown}: область имени `{area}` не совпадает с каталогом `{directory}`"
            ));
        }
    }

    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Имя файла правила переживает клонирование на Windows.
///
/// Windows занимает под устройства имена, оставшиеся от DOS: файл с таким именем там не
/// создаётся, и git не может развернуть дерево целиком. Прежде эта проверка дремала —
/// имена файлов начинались с `DEC.`, `INV.` или `CTR.`. Теперь имя называет тему свободным
/// словом, и `aux.md` или `con.md` — правдоподобный слог.
///
/// Перечень и правило разбора — по руководству Microsoft «Naming Files, Paths, and
/// Namespaces»: имя считается до первой точки (`NUL.tar.gz` — то же, что `NUL`), цифры у
/// `COM` и `LPT` — от единицы до девяти и надстрочные ¹, ², ³. `COM0` и `LPT0` в перечне нет.
/// Каталог — такое же имя, как файл.
#[test]
fn a_rule_name_survives_a_windows_checkout() {
    const RESERVED: &[&str] = &["con", "prn", "aux", "nul"];
    const NUMBERED: &[&str] = &["com", "lpt"];
    const DIGITS: &[&str] = &["1", "2", "3", "4", "5", "6", "7", "8", "9", "¹", "²", "³"];
    let is_reserved = |base: &str| {
        RESERVED.contains(&base)
            || NUMBERED.iter().any(|prefix| {
                base.strip_prefix(prefix)
                    .is_some_and(|digit| DIGITS.contains(&digit))
            })
    };
    let mut wrong = Vec::new();
    for path in rule_paths(&rules_root()) {
        for part in path
            .strip_prefix(rules_root())
            .unwrap_or(&path)
            .components()
        {
            let name = part.as_os_str().to_string_lossy().to_lowercase();
            let base = name.split('.').next().unwrap_or_default();
            if is_reserved(base) {
                wrong.push(format!(
                    "{}: Windows занимает `{base}` под устройство",
                    shown(&path)
                ));
            }
        }
    }

    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

mod guardrail_support;

use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};

use guardrail_support::{
    collect_rust_files, free_function_tokens, production_source, production_tokens,
    trait_impl_method_tokens,
};

const EXPECTED_MCP_TOOLS: &[&str] = &[
    "run_all_tests",
    "run_module_tests",
    "build_project",
    "dump_config",
    "launch_app",
    "check_syntax_edt",
    "check_syntax_designer_config",
    "check_syntax_designer_modules",
];

const FORBIDDEN_PROCESS_PATTERNS: &[&str] = &[
    "std::process::Command",
    "tokio::process::Command",
    "usestd::process::Command",
    "usestd::process::{Command",
    "usestd::process::Stdio",
    "usestd::process::{Stdio",
    "usestd::process::Child",
    "usestd::process::{Child",
    "usestd::process::ExitStatus",
    "usestd::process::{ExitStatus",
    "Command::new(",
    "Stdio::",
];

fn repo_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn read(relative: &str) -> String {
    fs::read_to_string(repo_path(relative)).expect("read repository file")
}

fn extract_between<'a>(contents: &'a str, start_marker: &str, end_marker: &str) -> &'a str {
    let start = contents
        .find(start_marker)
        .unwrap_or_else(|| panic!("missing marker: {start_marker}"));
    let tail = &contents[start..];
    let end = tail
        .find(end_marker)
        .unwrap_or_else(|| panic!("missing marker: {end_marker}"));
    &tail[..end]
}

fn extract_backticked_items(section: &str) -> Vec<String> {
    let regex = Regex::new(r"`([^`]+)`").expect("regex");
    regex
        .captures_iter(section)
        .map(|capture| capture[1].to_owned())
        .collect()
}

#[test]
fn raw_process_spawn_apis_stay_inside_platform_layer() {
    let root = repo_path("src");
    let files = collect_rust_files(&root);
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src_main = Path::new("src").join("main.rs");
    let src_platform = Path::new("src").join("platform");

    for file in files {
        let relative = file.strip_prefix(repo_root).expect("relative path");
        if relative == src_main || relative.starts_with(&src_platform) {
            continue;
        }

        let production = production_tokens(&file);
        for forbidden in FORBIDDEN_PROCESS_PATTERNS {
            assert!(
                !production.contains(forbidden),
                "{} must keep raw process API '{}' inside src/platform",
                relative.display(),
                forbidden
            );
        }
    }
}

#[test]
fn mcp_surface_snapshot_stays_explicit_and_documented() {
    let source = read("src/mcp/server.rs");
    let source_section = extract_between(
        &source,
        "const fn as_str(self) -> &'static str {",
        "fn admission_timeout",
    );
    let source_tools = Regex::new(r#""([a-z_]+)""#)
        .expect("regex")
        .captures_iter(source_section)
        .map(|capture| capture[1].to_owned())
        .collect::<Vec<_>>();

    let contract = read("spec/rules/mcp/published-tool-surface.md");
    // Числительное прозы сверяется со счётом, а не служит якорем: прежде девятый
    // инструмент с обновлённым перечнем и прежней прозой проходил молча.
    let published = contract
        .lines()
        .find(|line| line.starts_with("Опубликованы ") && line.ends_with(" инструментов:"))
        .expect("contract announces the published tools");
    let counted = published
        .trim_start_matches("Опубликованы ")
        .trim_end_matches(" инструментов:");
    let contract_section = extract_between(
        &contract,
        published,
        "Состав меняется только вместе с версией этой формы.",
    );
    let contract_tools = extract_backticked_items(contract_section);
    assert_eq!(
        counted,
        russian_numeral(contract_tools.len()),
        "проза называет другое число инструментов, чем перечисляет"
    );

    let expected = EXPECTED_MCP_TOOLS
        .iter()
        .map(|tool| (*tool).to_owned())
        .collect::<Vec<_>>();

    assert_eq!(source_tools, expected);
    assert_eq!(contract_tools, expected);
}

#[test]
fn public_command_adapters_keep_workspace_lock_boundary() {
    for function in [
        "execute_extensions",
        "execute_init",
        "execute_build",
        "execute_test",
        "execute_load",
        "execute_dump",
        "execute_infobase_configuration_export",
        "execute_infobase_dump",
        "execute_convert",
        "execute_artifacts",
        "execute_syntax",
        "execute_launch",
    ] {
        let window = free_function_tokens(repo_path("src/cli/execute.rs").as_path(), function);
        assert!(
            window.contains("with_cli_workspace_lock(")
                || window.contains("with_cli_workspace_lock_observed("),
            "{function} must keep the CLI workspace-lock boundary"
        );
    }

    for function in [
        "build_project",
        "run_tests",
        "dump_config",
        "launch_app",
        "check_syntax",
    ] {
        let window = trait_impl_method_tokens(
            repo_path("src/mcp/port.rs").as_path(),
            "McpUseCasePort",
            "DefaultMcpUseCasePort",
            function,
        );
        assert!(
            window.contains("with_workspace_lock("),
            "{function} must keep the MCP workspace-lock boundary"
        );
    }
}

/// Числительное для счёта, который эта проверка сверяет с прозой. Перечень короткий
/// намеренно: он покрывает правдоподобный размер поверхности, а не русский язык.
fn russian_numeral(count: usize) -> &'static str {
    match count {
        5 => "пять",
        6 => "шесть",
        7 => "семь",
        8 => "восемь",
        9 => "девять",
        10 => "десять",
        11 => "одиннадцать",
        12 => "двенадцать",
        other => panic!("числительного для {other} в проверке нет — допишите"),
    }
}

/// Вложенные шаги не берут блокировку повторно: замок рабочего каталога живёт на
/// границе адаптера, а сценарии зовут друг друга через входы без замка. Второй захват
/// изнутри дал бы «занято» самому себе.
#[test]
fn nested_orchestration_never_acquires_the_workspace_lock_inside_use_cases() {
    let root = repo_path("src/use_cases");
    for file in collect_rust_files(&root) {
        // `workspace_lock.rs` реализует замок, `transport.rs` — граница адаптера, где он
        // берётся один раз за команду. Всё остальное в слое сценариев работает под ним.
        if matches!(
            file.file_name().and_then(|name| name.to_str()),
            Some("workspace_lock.rs") | Some("transport.rs")
        ) {
            continue;
        }
        let production = production_tokens(&file);
        assert!(
            !production.contains("acquire_workspace_lock("),
            "{} takes the workspace lock inside a use case; nested steps run under the caller's lock",
            file.display()
        );
    }

    // `test` строит перед прогоном тем же сценарием сборки, не выходя на границу адаптера.
    let run_tests = read("src/use_cases/run_tests/coordinator.rs");
    assert!(
        run_tests.contains("build_project::execute("),
        "run_tests must reuse the build use case directly, under the lock already held by the caller"
    );
}

/// Лимит одновременных вызовов общий для обоих транспортов: семафор допуска создаётся
/// в одном месте, и оба конструктора — stdio и http — приходят к нему одной дорогой.
/// Второй `Semaphore::new` означал бы второй лимит, о котором конфиг не знает.
#[test]
fn mcp_admission_is_built_once_and_shared_by_both_transports() {
    let source = read("src/mcp/server.rs");
    let production = production_tokens(repo_path("src/mcp/server.rs").as_path());
    assert_eq!(
        production.matches("Semaphore::new(").count(),
        1,
        "admission must be built in exactly one place"
    );

    for constructor in ["fn stdio(", "fn http("] {
        let start = source
            .find(constructor)
            .unwrap_or_else(|| panic!("{constructor} constructor is missing"));
        let body = &source[start..];
        let end = body[constructor.len()..]
            .find("\n    pub fn ")
            .map(|offset| offset + constructor.len())
            .unwrap_or(body.len());
        let window = &body[..end];
        assert!(
            window.contains("with_port(") || window.contains("Self::new("),
            "{constructor} must build the server through the shared constructor"
        );
        assert!(
            !window.contains("Semaphore::new("),
            "{constructor} builds its own admission limit"
        );
    }
}

/// Слово исхода в подписи узла пишет presenter, а не рендерер.
///
/// Пока его писал каждый рендерер сам, подпись расходилась со знаком: `syntax` называл
/// проверку успешной, имея предупреждение среди подробностей, `test` обещал
/// предупреждения, не имея их. Оба расхождения нашли тесты, а не правило. Теперь слово
/// и знак берутся из одного значения (`NodeMark`), и рендереру незачем их произносить.
#[test]
fn a_renderer_never_spells_the_outcome_word_itself() {
    const OUTCOME_WORDS: &[&str] = &["completed successfully", "completed with warnings"];
    let renderers = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/cli/execute.rs");
    let text = fs::read_to_string(&renderers).expect("renderers are readable");
    for word in OUTCOME_WORDS {
        assert!(
            !text.contains(word),
            "{}: the outcome word `{word}` belongs to the presenter; name the subject and let it pick the word and the sign together",
            renderers.display()
        );
    }
}

/// Показ команды маскирует один владелец — `platform::secrets`. Пока его писала каждая
/// поверхность сама, превью запуска печатало `Pwd=***`, а отказ настоящего запуска той
/// же базы — `Pwd=s3cret`, и в stderr, и в журнал действий: правило, у которого четыре
/// исполнителя, — это четыре разных правила. Признак повтора — модуль, который строит
/// показ запрошенного процесса сам, не позвав владельца.
#[test]
fn a_process_command_is_shown_only_through_the_secrets_owner() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let owner = repo_path("src/platform/secrets.rs");

    for file in collect_rust_files(&repo_path("src")) {
        if file == owner {
            continue;
        }
        let production = production_tokens(&file);
        // `cmd:` — это поле показа у `ProcessError`; вместе с типом запроса оно и
        // означает, что модуль показывает argv запрошенного процесса. Поле `command`
        // журнала само по себе не признак: им называют и имя команды CLI.
        let requests_a_process = production.contains("ProcessRequest")
            || production.contains("InteractiveProcessRequest");
        if !(requests_a_process && production.contains("cmd:")) {
            continue;
        }

        assert!(
            production.contains("render_masked_command"),
            "{} shows a process command and must take the string from platform::secrets",
            file.strip_prefix(repo_root)
                .expect("relative path")
                .display()
        );
    }
}

/// Ключ `builder` снят решением `DEC.2026-09-14.BUILDER-KEY-IS-REMOVED`: конфиг с ним
/// не проходит валидацию. `tests/provider_matrix.rs` держит отказ со стороны рантайма,
/// а здесь — со стороны поставляемого навыка: `SKILL/` читают в чужих проектах, и
/// вернувшееся туда упоминание снова научило бы агентов писать конфиг, который раннер
/// отвергает. Единственный владелец выбора исполнителя — `providers.<операция>`.
#[test]
fn the_shipped_skill_never_names_the_removed_builder_key() {
    let skill_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("SKILL");
    let mut offenders = Vec::new();
    let mut stack = vec![skill_root.clone()];

    while let Some(path) = stack.pop() {
        for entry in fs::read_dir(&path).expect("read SKILL directory") {
            let entry = entry.expect("SKILL directory entry");
            let entry_path = entry.path();
            if entry_path.is_dir() {
                stack.push(entry_path);
                continue;
            }
            // Расширение не фильтруется: ключ может вернуться и в yaml рядом с навыком,
            // а «то же самое под другим именем» — это ровно то, что ловит эта проверка.
            if entry_path.extension().is_none() {
                continue;
            }
            let text = fs::read_to_string(&entry_path).expect("read SKILL file");
            for (index, line) in text.lines().enumerate() {
                if line.contains("builder") {
                    offenders.push(format!(
                        "{}:{}: {}",
                        entry_path
                            .strip_prefix(&skill_root)
                            .unwrap_or(&entry_path)
                            .display(),
                        index + 1,
                        line.trim()
                    ));
                }
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "SKILL/ must not name the removed `builder` key; use `providers.<operation>` instead:\n{}",
        offenders.join("\n")
    );
}

/// Маскирование секретов имеет одного владельца, и второй набор правил рядом с ним —
/// то, как эта дыра появилась в прошлый раз.
///
/// `run_tests` вёл собственный словарь флагов: знал `/P` и `/N` и не знал `/WSP`, `/UC`,
/// `/AccessToken`. Предыдущий guard его не видел, потому что смотрел только на модули,
/// показывающие argv процесса, а этот чистил чужую прозу. Здесь признак другой: файл,
/// который сам пишет регулярное выражение по секретному ключу, обязан звать владельца.
#[test]
fn secret_masking_rules_live_with_their_owner() {
    let owner = repo_path("src/platform/secrets.rs");
    // Ключи взяты из словаря владельца: их появление в регулярном выражении и означает
    // «здесь маскируют секрет».
    const SECRET_KEY_MARKERS: &[&str] = &["/P", "pwd=", "password=", "/WSP", "/UC"];
    let mut offenders = Vec::new();

    for file in collect_rust_files(&repo_path("src")) {
        if file == owner {
            continue;
        }
        // `production_tokens` убирает пробелы целиком, поэтому `Regex :: new` из текста
        // токенов снова читается как `Regex::new`. Искомые ключи пробелов не содержат.
        let production = production_tokens(&file);
        let writes_a_secret_regex = production.contains("Regex::new")
            && SECRET_KEY_MARKERS
                .iter()
                .any(|marker| production.contains(marker));
        if !writes_a_secret_regex {
            continue;
        }
        // Нужен вызов, а не упоминание: `platform::secrets` встречается и в прозе
        // комментария, поэтому признаком делегирования служит путь к элементу.
        if !production.contains("platform::secrets::") {
            offenders.push(file.display().to_string());
        }
    }

    assert!(
        offenders.is_empty(),
        "these modules mask secrets with their own rules instead of calling \
         platform::secrets, which is the single owner:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn the_loopback_question_is_answered_in_one_place() {
    // Корень проблемы: «петлевой ли адрес» решали сравнением подстрок, и `127.evil.com`
    // с `127.0.0.1@evil.com` проходили проверку. Владелец ответа один — `support::authority`,
    // потому что только разбор адреса знает про userinfo, скобки IPv6 и запись байтов.
    let owner = repo_path("src/support/authority.rs");
    // Первыми идут формы, которыми ошибка и была написана: сравнение по подстроке
    // и по префиксу. Без них защита стерегла бы только аккуратные написания — те,
    // которые и так безобидны, — и молчала бы ровно про тот дефект, чьё имя носит.
    const LOOPBACK_DECISION_MARKERS: &[&str] = &[
        "starts_with(\"127",
        "starts_with(\"::1",
        "contains(\"127",
        "contains(\"localhost",
        "ends_with(\"localhost",
        // Целиком закрытые литералы: `"127.0.0.1:3000"` из значения по умолчанию
        // ни под один из них не подходит, а плечо `match` и `Some("127")` — да.
        "\"127.\"",
        "\"127\"",
        "\"localhost\"",
        "\"::1\"",
        ".is_loopback()",
    ];
    let mut offenders = Vec::new();

    for file in collect_rust_files(&repo_path("src")) {
        if file == owner {
            continue;
        }
        let production = production_tokens(&file);
        let decides_about_loopback = LOOPBACK_DECISION_MARKERS
            .iter()
            .any(|marker| production.contains(marker));
        if !decides_about_loopback {
            continue;
        }
        if !production.contains("support::authority::") {
            offenders.push(file.display().to_string());
        }
    }

    assert!(
        offenders.is_empty(),
        "these modules decide what a loopback address is on their own instead of calling \
         support::authority, which is the single owner:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn a_host_port_record_is_read_in_one_place() {
    // Корень проблемы: запись `host:port` резали по последнему двоеточию в двух местах —
    // у SSH-шлюза автономного сервера и у `attach` агента, — и скобки IPv6 оставались в
    // хосте, который затем не разрешался. Владелец чтения один — `support::authority`:
    // только разбор адреса знает про скобки, порт, IDNA и запрещённые символы. Свои
    // правила поверх (порт обязателен) модули добавляют к его ответу, а не к строке.
    let owner = repo_path("src/support/authority.rs");
    // Маркеры порождаются разбором сниппетов: так они совпадают с тем, как `syn` печатает
    // токены, и не зависят от пробелов. Первые — формы, которыми дефект был написан.
    // Известная дыра: `parse::<SocketAddr>()` — тоже читатель адреса, но у него есть
    // законные места (`mcp.http.bind_address` — числовой адрес привязки), и маркером он
    // станет вместе со своим allowlist.
    let markers: Vec<(String, &str)> = [
        "rsplit_once(':')",
        "rsplit_once(\":\")",
        "rsplitn(2, ':')",
        "rsplitn(2, \":\")",
        "rfind(':')",
        "rfind(\":\")",
        "split_once(':')",
        "split_once(\":\")",
    ]
    .into_iter()
    .map(|call| (cut_marker(call), call))
    .collect();
    // Строки `ключ: значение` режут по первому двоеточию законно: это не адреса. Список
    // может только сокращаться.
    const KEY_VALUE_LINE_READERS: &[&str] = &[
        "src/platform/extension_inventory.rs",
        "src/support/edt_project.rs",
    ];
    let mut offenders = Vec::new();

    for file in collect_rust_files(&repo_path("src")) {
        if file == owner {
            continue;
        }
        let relative = file
            .strip_prefix(repo_path(""))
            .expect("inside the repository")
            .to_string_lossy()
            .replace('\\', "/");
        let source = without_doc_comments(&production_source(&file));
        for (marker, call) in &markers {
            if !source.contains(marker.as_str()) {
                continue;
            }
            if call.starts_with("split_once") && KEY_VALUE_LINE_READERS.contains(&relative.as_str())
            {
                continue;
            }
            offenders.push(format!("{relative}: {call}"));
        }
    }

    assert!(
        offenders.is_empty(),
        "these modules cut a host:port record by a colon on their own instead of calling \
         support::authority::host_and_port_of_authority, which is the single owner:\n{}",
        offenders.join("\n")
    );

    // Запись allowlist, за которой больше нет `split_once` по двоеточию, устарела: список
    // может только сокращаться, и стареть молча ему нельзя.
    let split_once_markers: Vec<&String> = markers
        .iter()
        .filter(|(_, call)| call.starts_with("split_once"))
        .map(|(marker, _)| marker)
        .collect();
    let stale: Vec<&str> = KEY_VALUE_LINE_READERS
        .iter()
        .copied()
        .filter(|relative| {
            let source = without_doc_comments(&production_source(&repo_path(relative)));
            !split_once_markers
                .iter()
                .any(|marker| source.contains(marker.as_str()))
        })
        .collect();
    assert!(
        stale.is_empty(),
        "stale KEY_VALUE_LINE_READERS entries — the files no longer split a line by a colon, \
         remove them from the allowlist:\n{}",
        stale.join("\n")
    );
}

/// Токены вызова `.<call>` в том виде, в каком их печатает `syn`, без приёмника.
fn cut_marker(call: &str) -> String {
    let expression: syn::Expr =
        syn::parse_str(&format!("v.{call}")).unwrap_or_else(|_| panic!("parse {call}"));
    let rendered = quote::ToTokens::to_token_stream(&expression).to_string();
    rendered
        .strip_prefix("v . ")
        .unwrap_or_else(|| panic!("receiver in {rendered}"))
        .to_owned()
}

/// Убирает `# [doc = "..."]` из исходного вида токенов, оставляя сам код. Литерал
/// документации читается до неэкранированной закрывающей кавычки: `]` внутри прозы
/// (`[v6]:port`) — не конец атрибута.
fn without_doc_comments(source: &str) -> String {
    const OPENING: &str = "# [doc = ";
    let mut kept = String::with_capacity(source.len());
    let mut rest = source;

    while let Some(at) = rest.find(OPENING) {
        kept.push_str(&rest[..at]);
        let after = &rest[at + OPENING.len()..];
        let Some(literal) = string_literal_len(after) else {
            return kept;
        };
        match after[literal..].find(']') {
            Some(close) => rest = &after[literal + close + 1..],
            None => return kept,
        }
    }
    kept.push_str(rest);
    kept
}

/// Длина строкового литерала в начале `text`, с кавычками, если он там стоит.
fn string_literal_len(text: &str) -> Option<usize> {
    let mut chars = text.char_indices();
    let Some((_, '"')) = chars.next() else {
        return None;
    };
    let mut escaped = false;
    for (index, ch) in chars {
        match ch {
            '\\' if !escaped => escaped = true,
            '"' if !escaped => return Some(index + 1),
            _ => escaped = false,
        }
    }
    None
}

#[test]
fn the_http_listener_never_hands_out_a_cross_origin_permission() {
    // Проверка `Origin` у слушателя MCP мягкая нарочно: она пускает любой петлевой
    // порт и все имена из `mcp.http.allowed_hosts`. Держится это на том, что
    // межисточниковый запрос браузер гасит сам, не получив разрешения. Стоит выдать
    // его — и мягкость превратится в дыру: каждое имя из списка станет читаемым из
    // чужого источника. Поэтому разрешения нет нигде.
    //
    // Приметы сравниваются в нижнем регистре: `HeaderName` приводит имя к нему сам,
    // поэтому написание в исходнике роли не играет, а точное совпадение по регистру
    // пропустило бы рабочую выдачу.
    const CROSS_ORIGIN_GRANTS: &[&str] = &[
        "access-control-allow-origin",
        "access_control_allow_origin",
        "corslayer",
        "tower_http::cors",
    ];
    let mut offenders = Vec::new();

    for file in collect_rust_files(&repo_path("src")) {
        // Документация про правило — не нарушение правила: `///` попадает в токены
        // как `#[doc="..."]`, и без этого описать запрет рядом с кодом было бы нельзя.
        let production = without_doc_attributes(&production_tokens(&file)).to_ascii_lowercase();
        if CROSS_ORIGIN_GRANTS
            .iter()
            .any(|grant| production.contains(grant))
        {
            offenders.push(file.display().to_string());
        }
    }

    let manifest = read("Cargo.toml").to_ascii_lowercase();
    assert!(
        !manifest.contains("\"cors\"") && !manifest.contains("'cors'"),
        "Cargo.toml enables a CORS layer; the MCP Origin rule assumes none is ever built"
    );
    assert!(
        offenders.is_empty(),
        "these modules hand out a cross-origin permission, which turns the deliberately \
         lenient MCP Origin rule into a readable cross-origin surface:\n{}",
        offenders.join("\n")
    );
}

/// Убирает из токенов содержимое `#[doc="..."]`, оставляя сам код.
fn without_doc_attributes(tokens: &str) -> String {
    const OPENING: &str = "#[doc=";
    let mut kept = String::with_capacity(tokens.len());
    let mut rest = tokens;

    while let Some(at) = rest.find(OPENING) {
        kept.push_str(&rest[..at]);
        let after = &rest[at + OPENING.len()..];
        match after.find(']') {
            Some(close) => rest = &after[close + 1..],
            None => return kept,
        }
    }
    kept.push_str(rest);
    kept
}

#[test]
fn the_http_listener_is_never_served_without_its_host_check() {
    // Проверка имени хоста — слой поверх маршрутизатора, и снять её можно одной
    // строкой: сборка останется зелёной везде, кроме `tests/mcp_http.rs`, а тот
    // объявлен `#![cfg(unix)]` и до Windows не доезжает. Поэтому саму проводку
    // держит примета: тот, кто поднимает слушатель, обязан навесить слой.
    let window = free_function_tokens(repo_path("src/mcp/server.rs").as_path(), "serve_http");

    assert!(
        !window.is_empty(),
        "serve_http is gone from src/mcp/server.rs; this guard names the wrong function"
    );
    for required in [
        "axum::serve",
        "from_fn_with_state",
        "refuse_a_request_that_names_another_host",
    ] {
        assert!(
            window.contains(required),
            "serve_http no longer wires the host check ({required} is missing): a listener \
             built without it answers any name, which is the DNS-rebinding hole itself"
        );
    }
}

#[test]
fn a_command_carries_no_deadline_anywhere_it_could_be_put_back() {
    // Корень проблемы: общий срок на команду не защищал базу, а ломал её — истёкший срок
    // означал, что загрузка одного набора исходников уже зафиксирована, а следующий отказан
    // на безопасной точке, то есть конфигурация обновлена наполовину
    // (DEC.2026-09-20.A-COMMAND-HAS-NO-DEADLINE).
    //
    // Владелец ответа один — `ExecutionContext`: у него нет поля срока и нет способа его
    // поставить, поэтому вернуть срок можно только заведя это поле заново. Страж стоит на
    // самих именах, а не на поведении: тест поведения проходит и без срока, и со сроком,
    // который никто не выставил, и потому регресс пропустит.
    const BANNED_IN_CONTEXT: &[&str] = &["deadline", "remaining_budget", "TimedOut"];

    let context =
        without_doc_attributes(&production_tokens(&repo_path("src/use_cases/context.rs")));
    let offenders: Vec<&str> = BANNED_IN_CONTEXT
        .iter()
        .copied()
        .filter(|marker| context.contains(marker))
        .collect();
    assert!(
        offenders.is_empty(),
        "src/use_cases/context.rs names {offenders:?}: a command budget is back on the execution \
         context. A bound belongs to the step that declares it, not above it."
    );

    // Срок уже однажды воскресал под другим именем: остаток допускного бюджета укорачивал
    // предел шага EDT в MCP. Допускной срок обязан кончаться вместе с допуском.
    let server = without_doc_attributes(&production_tokens(&repo_path("src/mcp/server.rs")));
    assert!(
        !server.contains("remaining_timeout"),
        "src/mcp/server.rs computes a remainder of the admission budget: an admitted call must \
         run to its terminal outcome, and a step cap must not be shortened by the queue wait."
    );
}

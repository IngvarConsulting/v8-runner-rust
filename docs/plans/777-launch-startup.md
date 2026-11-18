> Приостановлено владельцем: Unica перенесла #3197 в v0.17 («ещё обсудим, может быть не будем делать»).
> Замеры Windows — раздел #777 в `references/1c/confirmed-runtime-measurements.md`; скрипт замера — `scripts/measure/startup_probe.py`.
> Незавершённое — в теле коммита `wip(launch): …` этой ветки. Открытые вопросы владельца: стоит ли подтверждение только для файловой базы на Windows
> (серверные базы, `/WS`, Linux, macOS всегда `unconfirmed`), или вместо этого проверять реквизиты до запуска, или наблюдать только ранний выход.

# План #777 — `launch --observe-startup-ms N`, launch-data v8 `startup`

Контракт согласован владельцем (через сессию «Макс», Unica): флаг `--observe-startup-ms N` у
`launch thin|thick|ordinary`; поле `startup{outcome: started|auth_failed|start_failed|closed|unconfirmed,
event_at: RFC3339 до секунды | null, exit_code: int | null}`; `ok: true` и при `auth_failed`/`start_failed`.
Серверная база (`/S`, `/WS`, кластер, автономный сервер) — `unconfirmed` сразу после пробы старта.
Журнал SQLite (`.lgd`) не разбираем (решение владельца): `unconfirmed` с советом перевести журнал в
последовательный формат. Замер — последний раздел references/1c/confirmed-runtime-measurements.md (#777).

## Факты замера (Windows 8.3.27, 8.5.4)

- Последовательный журнал: `1Cv8Log/1Cv8.lgf` (словарь: `{1,<uuid>,"<имя>",N}` пользователь,
  `{2,"<компьютер>",N}`, `{3,"<приложение>",N}`, `{4,"<событие>",N}`) и `<период>.lgp` с записями
  `{ГГГГММДДччммсс,<тр.статус>,{тр},<польз>,<комп>,<прил>,<соединение>,<событие>,...,{<данные>},...}`.
- Отказ: одно `_$Session$_.AuthenticationError`, пользователь 0; имя попытки в данных
  `{"P",{518,{"S","<имя>"},{"S","<ОС>"},{"N",1}}}` (стандартная) или `{"P",{513,{"S","<ОС>"},...}}` (ОС);
  событие пишется за 0,8–1,4 с, раньше выхода (код 1, 1,0–1,6 с).
- Успех: `_$Session$_.Authentication` и `_$Session$_.Start` с пользователем.
- Битая база: код 1, записей нет. Толстый, штатное закрытие: код 0.
- Тонкий с `/DisableStartupDialogs` может выйти с кодом 1 через 0,2 с после `Session.Start` (#784).
- PID в записи нет; время местное, до секунды.

## Устройство

1. `src/platform/event_log.rs` (новый модуль платформы, один владелец чтения журнала):
   - `EventLogMark::take(base_dir)` — до запуска: формат (`Sequential` по `1Cv8.lgf`, `Sqlite` по
     `1Cv8.lgd`, `Absent`), размеры всех `*.lgp`, размер `.lgf`.
   - `EventLogMark::read_new(&self) -> Result<Vec<LogEvent>, …>` — дочитать словарь целиком, из
     `*.lgp` (включая появившиеся после метки) только добавленное после метки; разбор записей
     скобочного формата 1С (строки в `"`, удвоенные кавычки, вложенные `{}`), неполная последняя
     запись не ошибка — отложить до следующего чтения.
   - `LogEvent { at: NaiveDateTime, event: String, user: Option<String>, computer: String,
     application: String, attempted_user: Option<String> }`.
   - Модуль тестируется на текстовых фикстурах (взять настоящие записи из замера).
2. `src/use_cases/launch_startup.rs` — наблюдение: вход — managed-процесс, метка журнала,
   ожидаемые `application` (`1CV8C` для thin, `1CV8` для thick/ordinary), компьютер (имя хоста,
   без учёта регистра), имя попытки (`/N` из реквизитов базы или `None`), окно N мс, политика
   отмены. Цикл опроса 200 мс:
   - процесс вышел → дочитать журнал до 1 с → ровно одно подходящее `AuthenticationError` и нет
     других подходящих событий входа → `auth_failed`; несколько подходящих событий → `unconfirmed`;
     код 0 → `closed`; иначе `start_failed`;
   - процесс жив, ровно одно подходящее `Session.Start` (и `Authentication`) нашей попытки, и
     клиент прожил после него ещё 2 с → `started` (досрочно, не дожидаясь N) — предложено Максу;
   - подходящих событий входа больше одного (чужая попытка того же имени, приложения и
     компьютера в окне) → `unconfirmed`;
   - окно истекло, клиент жив, решения нет → `unconfirmed`.
   - `event_at` — время решающего события (`AuthenticationError` или `Session.Start`), местное →
     RFC3339 со смещением зоны машины; иначе `null`. `exit_code` — код, если процесс вышел.
   - Клиент живёт дальше: managed-ручка отпускается (`detach`), как у `--wait-ready`.
   - Отмена команды во время окна: клиент снимается (`managed.cancel()`), ответ — отмена.
3. `launch_app.rs`: при `observe_startup` — `spawn_managed(Detached)` без пробы старта (иначе ранний
   выход стал бы отказом `ExitedEarly`, а не `start_failed`); для серверной базы — обычный путь с
   пробой и `startup{unconfirmed, null, null}`; файловая база: метка журнала до запуска.
4. Домен: `LaunchStartup { outcome: StartupOutcome, event_at: Option<DateTime<FixedOffset>>,
   exit_code: Option<i32> }`, `LaunchResult.startup: Option<…>` (`skip_serializing_if`).
5. CLI: `--observe-startup-ms <MS>` (u64 ≥ 1) в `LaunchArgs`; отказ `invalid_argument` до запуска
   для `designer`, `web`, `mcp`, вместе с `--wait-for-exit`, `--wait-ready` и с `--dry-run`
   (превью не запускает клиента — как у `--wait-for-exit`). MCP-поверхность launch не меняется
   (флаг только CLI, Unica зовёт CLI).
6. Замок: launch с наблюдением замок рабочей копии не берёт, параллельный `push` не ждёт;
   привязку доказывает фильтр, не замок (согласовано с Максом). Записать в правило.
7. Правила: `spec/rules/wire/launch-data.md` → version 8 («Что изменила версия 8»), схема
   `docs/schemas/command-data/launch.schema.json`; новое поведенческое правило
   `spec/rules/cli/a-launch-observes-its-own-startup.md` со `check:`; правило формата журнала в
   `spec/rules/platform/` (последовательный формат, SQLite — `unconfirmed`).
   Документы: CAPABILITIES (launch), SKILL (коротко), CONFIGURATION не меняется.
8. Тесты: модуль журнала на фикстурах (оба вида данных 518/513, кавычки, неполная запись, новый
   файл `.lgp`, `.lgd` → Sqlite); `launch_startup` на заглушке клиента (скрипт, который пишет
   записи в `1Cv8Log` и выходит с кодом / живёт): auth_failed, started, start_failed, closed,
   unconfirmed (две попытки; окно истекло; `.lgd`; серверная база сразу), отказы флага;
   отмена во время окна. Тесты на заглушках — unix-скрипты; Windows — модульные тесты журнала.
9. Живой прогон Windows 8.3.27: `launch thick --observe-startup-ms 15000` с верным, неверным
   паролем, битой базой; `launch thin` (ожидаем `start_failed` из-за #784); база по `/S` →
   `unconfirmed` сразу. Время ответа.

## Решения после скептика (обязательны, заменяют противоречащие пункты выше)

- Владение (C1): запуск с наблюдением идёт тем же режимом, что обычный `launch` (`ProcessIoMode::Detached` с
  `with_client_owner`, т.е. `UnicaOwnedDetached` при `V8_RUNNER_CLIENT_OWNER=unica`), без своей группы и Job.
  Новый метод `ProcessRunner::spawn_observed(request, work) -> ObservedSpawn` (по умолчанию Unsupported, как
  `spawn_managed`): хранит `SpawnedChild` только для `try_wait`; `Drop` клиента НЕ снимает; `release()` отдаёт
  `SpawnResult`; `kill()` для отмены — снять только сам процесс (без группового сигнала при host-owned).
  Без пробы старта (иначе ранний выход стал бы `ExitedEarly`). Проверка в `check:` правила host-owned-client.
- Замок (C2): граница `launch` не меняется (замок рабочей копии, памяти, базы, проверка владельца,
  `settle_live_agents` — до запуска) и держится всё окно наблюдения; параллельный `push` ждёт до
  окончания наблюдения (это согласовано владельцем через Макса). Записать в поведенческое правило.
- ОС (решение владельца): наблюдение действует только на Windows. На Linux и macOS — сразу после пробы старта
  `startup{unconfirmed, null, null}` (как серверная база), пробел #785 в правиле и CAPABILITIES.
- Серверная база (адрес не `File=`, либо `--via web`): обычный путь с пробой старта, затем сразу
  `startup{unconfirmed, null, null}`. Ранний выход у неё — прежний отказ `ExitedEarly`; расхождение с
  файловой базой назвать в правиле.
- Ключи подключения/пользователя в `--raw-key` или `tools.enterprise.additional-launch-keys`
  (`/F`, `/S`, `/WS`, `/IBConnectionString`, `/N`, `/P`, `/WA`, без учёта регистра) → сразу `unconfirmed`.
- Таблица решения (чистая функция `decide(events, exit, now_since_start_seen, …)`, тестируется на всех ОС):
  - «наши» события: приложение = ожидаемое (`1CV8C` thin, `1CV8` thick/ordinary; сравнение через
    `ApplicationCode` / `session_list::application_of_rac`), компьютер — `domain::sessions::from_this_machine`,
    пользователь ОС из данных события = текущий пользователь ОС (без учёта регистра; на Windows `ДОМЕН\имя`
    → сравнивать часть после `\`), имя попытки = `/N` из реквизитов базы (без учёта регистра) или, без `/N`,
    только вид данных 513 (вход по ОС). `Authentication`, `Start`, `Finish`, `AccessDenied` связываются с
    попыткой по номеру соединения (и сеанса) записи.
  - Любое событие `_$Session$_.Authentication*` в окне, не подходящее под фильтр, или больше одной нашей
    попытки → `unconfirmed`. Нечитаемая запись, незнакомый номер словаря (после дочитывания словаря),
    незнакомая форма данных, укоротившийся файл, незнакомый заголовок, `.lgd` (в том числе вместе с `.lgf`)
    → `unconfirmed`. Ошибка ввода-вывода журнала → `unconfirmed`, клиент не трогать.
  - Процесс вышел: дочитывать журнал до 1 с. Ровно одна наша попытка с `AuthenticationError` → `auth_failed`
    (`event_at` = её время). Наша попытка с `Start`, затем выход: код 0 → `closed`, иначе `start_failed`
    (`event_at` = время `Start`). Ни одного события входа ни от кого: код ≠ 0 → `start_failed`
    (`event_at: null`), код 0 → `unconfirmed`.
  - Процесс жив: наша попытка с `Start` без `AccessDenied`/`Finish` этого соединения, и с момента, когда
    раннер увидел `Start`, прошло ≥ 2 с → `started` (досрочно). Конец окна без решения → `unconfirmed`.
  - `exit_code` — код, если вышел, иначе `null`.
- Журнал (M3): заголовок `1CV8LOG(ver 2.0)`; словарь и `.lgp` читаются по смещениям (словарь тоже), запись
  полна по балансу `{}` с учётом строк в кавычках (`""` — кавычка, `\` — литерал, внутри строк бывают
  переводы строк); ведущая `,` и пробелы между записями допустимы; хранится смещение конца последней полной
  записи; UTF-8 декодируется только по полным записям; новые `*.lgp`, появившиеся после метки, читаются с
  начала; если в момент метки журнала нет — новые файлы читаются с нуля.
- Время: местное `ГГГГММДДччммсс` → RFC3339 через `chrono::Local`; неоднозначный час — `earliest()`,
  несуществующий — `null`.
- Один владелец скобочного формата 1С (M1): новый модуль `src/platform/brackets.rs` (разбор значения
  `{…}`/строк/атомов); `file_base_hold::lock_message` переводится на него; guard в
  `tests/architecture_guardrails.rs`: удвоенные кавычки 1С разбирает только `brackets.rs`.
- Ошибка после запуска клиента (M2) не убивает клиента: только `unconfirmed` и `release`.
- Отмена во время окна (M7): клиент снимается (`kill`), ответ — форма `launch` через
  `UseCaseFailure::after_possible_work`, `provider_dispatched: true`, без `startup`.
- N: 1..=600000 (больше — `invalid_argument`). Флаг только для `thin|thick|ordinary`; вместе с
  `--wait-for-exit`, `--wait-ready`, `--dry-run` и для `designer|web|mcp` — `invalid_argument` до запуска.
- Текст ответа (`render_launch_text`) и `message` называют исход.
- `LaunchResult.ok` doc-comment поправить: при наблюдении `ok: true` и при `auth_failed`/`start_failed`.

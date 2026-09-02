# ADR-0023: Типизировать экспорт конфигурации и снимка информационной базы

- Статус: `accepted`
- Дата: `2026-09-02`
- Связанные решения: [ADR-0001](0001-granitsy-podderzhki-ibcmd-kak-ogranichennogo-backend.md), [ADR-0005](0005-razdelit-cli-i-mcp-publichnye-poverhnosti.md), [ADR-0006](0006-sohranyat-transportno-neytralnyy-use-case-sloy.md), [ADR-0008](0008-derzhat-platformennye-backend-dsl-otdelno-ot-orchestration.md), [ADR-0011](0011-eksklyuzivnoe-vladenie-workpath-na-vremya-komandy.md), [ADR-0014](0014-edinaya-timeout-cancellation-policy-dlya-cli-i-mcp-komand.md), [ADR-0015](0015-atomarnaya-publikatsiya-dump-artifacts-cherez-staging-backup.md), [ADR-0016](0016-edinyy-executionoutcome-i-pipeline-steps-dlya-runner-like-stsenariev.md)

## Контекст

`dump` выгружает ИБ в XML/EDT source-set, а `make` строит CF/CFE/EPF/ERF из
исходников. Ни одна из этих команд не выражает два других намерения:

1. сохранить текущее состояние конфигурации ИБ в CF/CFE;
2. сохранить всю ИБ вместе с данными в переносимый DT-файл.

Платформа различает working configuration, database configuration, extension
и полный снимок ИБ. Сведение их в один generic artifact operation заставляет
вызывающего угадывать источник правды, destructive semantics и ожидаемое
расширение файла.

## Решение

Ввести два transport-neutral use case:

```text
ExportConfigurationPackage {
  state: Working | Database,
  subject: Main | Extension { name },
  output
} -> CF | CFE

ExportInfobaseSnapshot { output } -> DT
```

Они публикуются только в CLI; MCP surface runner не расширяется. Unica может
вызывать CLI как внутренний adapter, не копируя платформенные команды в свой
публичный контракт.

Точная CLI grammar:

```text
v8-runner infobase configuration export \
  --state <working|database> \
  [--extension <name>] \
  --output <path.cf|path.cfe>

v8-runner infobase dump --output <path.dt>
```

Canonical command identities: `infobase.configuration.export` и
`infobase.dump`. `--extension` отсутствует для основной конфигурации и задаёт
subject extension при наличии. Для main обязателен `.cf`, для extension —
`.cfe`, для снимка — `.dt`; регистр суффикса несущественен. Пустое или не
являющееся идентификатором 1С имя extension, неизвестный `state`, неверный
суффикс и параметры не своей команды отклоняются до workspace lock и provider
dispatch. Публичного `--provider`/`--engine` нет.

Обе команды используют command-specific config validation: обязательны project/base path,
`workPath`, `infobase`, применимые IBCMD DBMS-поля, platform version и общий execution timeout,
но build/source/test/EDT/client-MCP настройки не являются входами операции. Поэтому существующий
`v8project.yaml` с `source-set: []` допустим для этих двух команд; schema и full-validation
остальных команд не ослабляются.

`--json-message` использует существующий envelope
`ok/command/duration_ms/data/warnings/steps/error?`. Успешный и business-failure
payload сохраняет закрытые поля:

```text
data = {
  state,                         # только configuration export
  subject: {kind: main} | {kind: extension, name},
  provider: designer-batch | ibcmd-process | null,
  evidence: available | unverified | unsupported,
  provider_reason,
  artifact_kind: cf | cfe | dt,
  output,
  published,
  execution: {status, diagnostics?, errors?, interruptions?}
}
```

Для `infobase.dump` `state` отсутствует, а `subject={kind: infobase}`. До
provider selection `provider=null`; после выбора provider и основание остаются
видимыми и при terminal failure. Text presenter сообщает те же факты без
отдельной схемы. `warnings` и `steps` живут только в общих верхнеуровневых
каналах envelope и не дублируются внутри `data`.

`execution.status` различает `succeeded`, `failed`, `cancelled`, `timed_out` и
`invalid_output`; ошибка несёт stable code и interruption metadata. Для
`ExecutionError.retryable` действует общий serde default `false`: отсутствие
поля означает запрет автоматического retry. Не прошедшая capability selection
возвращает `error.code=capability_unavailable`, `error.kind=capability`, а не
`invalid_argument`: менять корректные аргументы в этом случае не нужно.

### Capability evidence

Поддержка provider имеет три состояния:

- `available`: operation/version/transport подтверждены реализацией и тестом;
- `unverified`: команда известна из help/spec, но execution/artifact channel не доказан;
- `unsupported`: provider явно не может исполнить семантическую операцию.

Только `available` участвует в выборе. Help text, command manifest или наличие
binary сами по себе не переводят capability в `available`.

Первый slice допускает только уже существующие process providers:

- Designer batch;
- IBCMD.

Designer Agent, `ibcmd-rs` и другие будущие providers остаются `unverified` до
отдельного lifecycle/credentials/artifact-transfer контракта и live proof.

Начальная ordered capability matrix фиксирована и не зависит от порядка
регистрации:

| Intent | `builder=DESIGNER` | `builder=IBCMD` |
| --- | --- | --- |
| working/database main CF | `designer-batch`, available | `ibcmd-process`, available |
| working/database extension CFE | `designer-batch`, available | `ibcmd-process`, available |
| infobase DT, file/server | `designer-batch`, available | `ibcmd-process`, unverified |

Evidence для `available`: Designer batch argv-tests на `/DumpCfg`,
`/DumpDBCfg`, `-Extension` и `/DumpIB`; IBCMD argv-tests на
`infobase config save <stage>`, optional `--db`/`--extension`; use-case artifact
validation и publication tests. Само наличие команды `ibcmd infobase dump` в
справке не достаточно: документация требует отсутствия подключений и
предупреждает о возможном нарушении целостности DT, поэтому этот кандидат не
становится available без проверяемого session/exclusive-access preflight.

### Selection и совместимость

Существующий `builder` остаётся строгим backend selector и для новых use case:
он выбирает ровно одну колонку матрицы, а не fuzzy preference. При
`builder=IBCMD` runner не переходит на Designer, и наоборот. Существующие
команды обязаны выбирать прежний backend и сохранять прежние CLI и JSON
envelopes. Новый публичный config selector в этом slice не вводится.

Pre-dispatch строго состоит из чтения выбранной колонки, проверки capability,
binary/version/connection и захвата locks. Вызов provider adapter является
commit point: после него нет fallback, включая ошибку `spawn()`. Auth/license
failure, timeout, cancellation, неизвестный terminal outcome, platform failure,
transfer или publication failure также не запускают другой provider. Future
selector с несколькими кандидатами обязан определить новый порядок отдельным
решением; текущий порядок всегда одноэлементный.

### Publication и lifecycle

Provider пишет только в private sibling staging path. Use case проверяет, что
артефакт является обычным непустым файлом ожидаемого типа, и публикует его через
общий staging/backup owner из ADR-0015. `published=true` появляется только после
успешного final publish. Старый target гарантированно не меняется до publish и
сохраняется при provider/validation failure. При publish failure обязательна
попытка rollback; если rollback не удался, failed outcome явно сообщает, что
target требует ручной проверки. Cleanup failure считается degraded success
только после успешного publish.

Обе команды владеют одним workspace lock, target-specific advisory lock и одним
execution deadline от selection до cleanup. Порядок захвата всегда
`workspace lock -> target lock`; target lock берётся до staging/orphan cleanup и
удерживается до окончания publication cleanup. Ожидание target lock проверяет
общий deadline/cancellation, а identity target повторно разрешается под lock
непосредственно перед publish. Это сериализует публикацию в
один output даже из разных workspace. DT — переносимый снимок, а не резервная
копия. IBCMD DT не становится `available`, пока runner не доказывает отсутствие
активных подключений либо иной безопасный exclusive-access contract.

## Неграницы

1. Не добавлять MCP tools runner.
2. Не загружать CF/CFE и не восстанавливать DT в этом изменении.
3. Не добавлять Designer Agent transport/session.
4. Не менять `builder`, schema `v8project.yaml` или поведение существующих команд.
5. Не выполнять fallback после spawn и не выдавать неизвестный outcome за retryable.
6. Не называть DT backup и не обещать его целостность без exclusive-access proof.

## Последствия

1. `make`, source `dump`, configuration package export и infobase snapshot имеют
   разные имена и результаты.
2. Новые providers могут подключаться к semantic contract без изменения CLI.
3. IBCMD CF/CFE использует доказанную команду `config save`; IBCMD DT остаётся
   честно недоступным до session preflight.
4. Reverse operations получат отдельные ADR и destructive acceptance matrix.

## План реализации

1. Добавить domain request/result и capability evidence vocabulary.
2. Добавить Designer batch DSL для `/DumpDBCfg` и `/DumpIB`; переиспользовать `/DumpCfg`.
3. Добавить IBCMD DSL для `config save`; `infobase dump` сначала оставить `unverified`.
4. Реализовать два use case с `ExecutionOutcome`, workspace lock и общей публикацией.
5. Добавить CLI group `infobase`, JSON/text presenters и compatibility snapshots.
6. Обновить `docs/CAPABILITIES.md`, `docs/CONFIGURATION.md`, `SKILL/SKILL.md` и acceptance.

## Верификация

- [x] Existing command parse/output snapshots не изменились.
- [x] Working/main экспорт вызывает `/DumpCfg` или `ibcmd config save`.
- [x] Database/main экспорт вызывает `/DumpDBCfg` или `ibcmd config save --db`.
- [x] Extension экспорт формирует CFE и передаёт точное имя extension.
- [x] DT через Designer batch вызывает `/DumpIB` и не называется backup.
- [x] Provider failure до publish сохраняет прежний target.
- [x] Два workspace, публикующие один output, сериализуются target lock.
- [ ] Publish failure пытается выполнить rollback; rollback failure требует ручной проверки target.
- [x] Cross-provider retry после spawn отсутствует.
- [x] Parse/help snapshots фиксируют точную grammar и отклоняют неверные сочетания flags/suffixes.
- [x] Text и `--json-message` фиксируют command identity, provider reason и `published` без противоречий.
- [x] Timeout/cancellation/invalid output имеют разные terminal statuses и non-retryable errors.
- [x] Target lock соблюдает deadline, а identity повторно проверяется перед publish.
- [x] Infobase-only config с `source-set: []` не блокируется source validation.
- [x] MCP tool list не изменился.

# ADR-0024: Типизировать экспорт конфигурации и снимка информационной базы

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
`workPath`, `infobase` и общий execution timeout. Версия платформы необязательна, а её
применение наследует общий locator contract: без explicit `path` она фильтрует обычный поиск,
с `path` проверяется при `strict: true` и игнорируется при `strict: false`. Build/source/test/EDT/
client-MCP настройки не являются входами операции. Поэтому существующий
`v8project.yaml` с `source-set: []` допустим для этих двух команд; schema и full-validation
остальных команд не ослабляются.

IBCMD DBMS-поля проверяются только в readiness этого кандидата. Их отсутствие не блокирует
Designer alternate: иначе `builder` снова становился бы строгим селектором до provider selection.
Для file connection readiness каждого process provider также требует существующий обычный файл
`<infobase>/1Cv8.1CD`; наличие только каталога ИБ или platform binary недостаточно.

`--json-message` использует существующий envelope
`ok/command/duration_ms/data/warnings/steps/error?`. Успешный и business-failure
payload сохраняет закрытые поля:

```text
data = {
  state,                         # только configuration export
  subject: {kind: main} | {kind: extension, name},
  selection: {
    provider: designer-batch | ibcmd-process | null,
    reason,
    candidates: [{
      provider,
      implementation: implemented | experimental | unsupported,
      readiness: ready | unavailable | not_checked,
      evidence: documented | argv_tested | live_verified,
      reason
    }]
  },
  artifact_kind: cf | cfe | dt,
  output,
  published,
  target_state: unchanged | created | replaced | restored | uncertain,
  execution: {status, diagnostics?, errors?, interruptions?}
}
```

Для `infobase.dump` `state` отсутствует, а `subject={kind: infobase}`. До
provider selection `selection.provider=null`; после выбора provider, список кандидатов и основание остаются
видимыми и при terminal failure. Text presenter сообщает те же факты без
отдельной схемы. `warnings` и `steps` живут только в общих верхнеуровневых
каналах envelope и не дублируются внутри `data`.

`execution.status` различает `succeeded`, `failed`, `cancelled`, `timed_out` и
`invalid_output`; ошибка несёт stable code и interruption metadata. Для
`ExecutionError.retryable` действует общий serde default `false`: отсутствие
поля означает запрет автоматического retry. Если для точной комбинации операции нет
implemented adapter, selection возвращает `capability_unavailable`. Если adapter реализован,
но pure preflight не нашёл готового binary/version/connection, возвращается
`environment_unavailable`. Эти ошибки не являются `invalid_argument`.

### Implementation, evidence и readiness

Implementation имеет три состояния:

- `implemented`: runner имеет adapter для точной operation matrix и разрешает dispatch;
- `experimental`: команда известна, но обязательный safety/lifecycle contract не завершён;
- `unsupported`: provider явно не может исполнить семантическую операцию.

`implemented` не означает live proof конкретного окружения. Поле evidence показывает
максимальную доказанность: `documented`, `argv_tested`, `live_verified`. Readiness независимо
показывает pure preflight текущего окружения: `ready`, `unavailable`, `not_checked`. Только
`implemented + ready` участвует в выборе; `experimental` по умолчанию не dispatch-ится.

Первый slice допускает только уже существующие process providers:

- Designer batch;
- IBCMD.

Designer Agent, `ibcmd-rs` и другие будущие providers остаются `experimental` до
отдельного lifecycle/credentials/artifact-transfer контракта и live proof.

Начальная implementation matrix фиксирована по operation × state × subject × topology ×
platform version × OS. `evidence` описывает уровень проверки адаптера в репозитории, а не
готовность текущей машины; её отдельно сообщает `readiness`. Текущий slice использует
документированные cross-platform команды и
argv/artifact tests; отсутствие live proof честно отражается evidence=`argv_tested`:

| Intent | `designer-batch` | `ibcmd-process` |
| --- | --- | --- |
| working/database main CF | implemented | implemented |
| working/database extension CFE | implemented | implemented |
| infobase DT, file/server | implemented | experimental |

Evidence=`argv_tested`: Designer batch argv-tests на `/DumpCfg`,
`/DumpDBCfg`, `-Extension` и `/DumpIB`; IBCMD argv-tests на
`infobase config save <stage>`, optional `--db`/`--extension`; use-case artifact
validation и publication tests. Само наличие команды `ibcmd infobase dump` в
справке не достаточно: документация требует отсутствия подключений и
предупреждает о возможном нарушении целостности DT, поэтому этот кандидат не
становится implemented без проверяемого session/exclusive-access preflight.

### Selection и совместимость

Для новых export use case существующий `builder` задаёт только первого кандидата. Runner
проверяет operation-specific candidates в детерминированном порядке и может выбрать alternate,
если preferred candidate не implemented или pure preflight пометил его unavailable. Поэтому DT
может использовать Designer при `builder=IBCMD`, а CF/CFE — Designer при отсутствующем IBCMD.
Поведение существующих команд не меняется; нового публичного selector нет.

Канонический порядок: request validation до чтения config; minimal non-mutating config
validation; capability/readiness selection; workspace lock; target lock; metadata-authenticated
orphan cleanup; staging; dispatch. Preflight может только парсить config/connection и читать
filesystem через locator: он не запускает процессы, не создаёт workPath, логи, locks, staging или
output parent. Первый spawn provider является commit point: после него нет fallback, включая
исчезновение binary между preflight и `spawn()`. Auth/license
failure, timeout, cancellation, неизвестный terminal outcome, platform failure,
transfer или publication failure также не запускают другой provider.

### Publication и lifecycle

Provider пишет только в private sibling staging path. Под target lock общий owner удаляет только
старые staging/backup с валидной metadata, совпадающими tool и target identity. Use case проверяет, что
артефакт является обычным непустым файлом ожидаемого типа, и публикует его через
общий staging/backup owner из ADR-0015. `published=true` появляется только после
успешного final publish. Старый target гарантированно не меняется до publish и
сохраняется при provider/validation failure. При publish failure обязательна
попытка rollback; если rollback не удался, failed outcome явно сообщает, что
target требует ручной проверки. Cleanup failure считается degraded success только после
успешного publish. `target_state` различает отсутствие commit (`unchanged`), первую публикацию
(`created`), замену (`replaced`), успешный rollback (`restored`) и недоказанное состояние после
rollback failure (`uncertain`). `uncertain` всегда требует ручной проверки target.

Обе команды владеют одним workspace lock, target-specific advisory lock и одним command execution
deadline от capability/readiness selection до cleanup. Pure selection не запускает процессы и не
ждёт locks, но учитывает тот же cancellation/deadline между проверками кандидатов. Порядок захвата всегда
`workspace lock -> target lock`; target lock берётся до staging/orphan cleanup и
удерживается до окончания publication cleanup. Held workspace lock возвращает `workspace_busy`.
Новые процессы дополнительно сериализуются OS file lock, а owner metadata публикуется атомарно.
Оставшийся после crash owner lock не удаляется автоматически: совместимость с legacy writer требует
fail-closed и offline cleanup после подтверждения, что процессы runner остановлены.
Ожидание target lock возвращает `timed_out` или `cancelled`; те же codes используются после spawn
без fallback. Внешний `error.code`, `execution.status` и nested error code совпадают. Identity
target повторно разрешается под lock
непосредственно перед publish. Это сериализует публикацию в
один output даже из разных workspace. DT — переносимый снимок, а не резервная
копия. IBCMD DT не становится `implemented`, пока runner не доказывает отсутствие
активных подключений либо иной безопасный exclusive-access contract.

Гарантия сериализации относится к cooperating запускам v8-runner, использующим этот advisory
lock. Ревалидация обнаруживает подмену пути до commit, но не является filesystem CAS и не может
запретить постороннему процессу менять target в узком окне самого commit. Интеграция с внешним
publisher обязана координировать запись тем же lock или не запускаться параллельно; значения
`created/replaced` не являются доказательством при нарушении этой границы.

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
3. IBCMD CF/CFE использует реализованную команду `config save`; IBCMD DT остаётся
   experimental до session preflight, а runner выбирает готовый Designer, если он доступен.
4. Reverse operations получат отдельные ADR и destructive acceptance matrix.

## План реализации

1. Добавить domain request/result и implementation/readiness/evidence vocabulary.
2. Добавить Designer batch DSL для `/DumpDBCfg` и `/DumpIB`; переиспользовать `/DumpCfg`.
3. Добавить IBCMD DSL для `config save`; IBCMD DT сначала оставить `experimental`.
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
- [x] Реальная ветка publish failure восстанавливает прежние байты target и возвращает `target_state=restored`.
- [x] Fault-injection всей ветки rollback failure подтверждает сохранённые stage/backup, отсутствие недостоверного target и `target_state=uncertain`.
- [x] Cross-provider retry после spawn отсутствует.
- [x] Parse/help snapshots фиксируют точную grammar и отклоняют неверные сочетания flags/suffixes.
- [x] Text и `--json-message` фиксируют command identity, provider reason и `published` без противоречий.
- [x] Timeout/cancellation/invalid output имеют разные terminal statuses и non-retryable errors.
- [x] Target lock соблюдает deadline, а identity повторно проверяется перед publish.
- [x] Infobase-only config с `source-set: []` не блокируется source validation.
- [x] Provider selection завершается до workspace lock и filesystem side effects.
- [x] Preferred provider допускает только pre-spawn fallback; после dispatch fallback отсутствует.
- [x] Outer error code совпадает с terminal/nested code для invalid output и interruption.
- [x] MCP tool list не изменился.

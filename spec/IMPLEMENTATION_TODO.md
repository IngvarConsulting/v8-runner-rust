# Active TODO For `v8-runner`

This file tracks open implementation work only.

## Current Status

- Open tasks as of `2026-09-14`: 13.

## Open Tasks

1. Перевести IBCMD DT provider из `experimental` в `implemented` только после реализации и live
   proof проверяемого no-active-connections или exclusive-access preflight; readiness при этом
   остаётся отдельной проверкой конкретного окружения. Это относится и к DT export, и к
   `infobase restore`; адаптер IBCMD для загрузки добавляется вместе с preflight, а не раньше.
2. Решить, как выглядит принудительное завершение сеансов для `infobase restore`: у IBCMD есть
   `--force` и `--session-terminate-message`, у Designer `/RestoreIB` такого ключа нет. Нужен
   публичный контракт расхождения возможностей, а не ключ, работающий у одного провайдера.
3. Дополнить `extensions ... update` до полного набора свойств платформы: сейчас ставятся два из
   шести (`--safe-mode`, `--unsafe-action-protection`), а платформа принимает ещё
   `--security-profile-name`, `--used-in-distributed-infobase` и `--scope`. **Поправка:** прежняя
   редакция утверждала, что `--active` вынесен отдельной подкомандой `activate`; это неверно.
   Документация (Приложение 4, 4.10.4.7.12) перечисляет `--active` среди параметров `update`, а
   подкоманды `activate` нет вовсе — замерено на 8.3.27.2074: `ibcmd config extension update
   --name=X --active=no` проходит и список показывает `active : no`, а `extension activate` парсер
   отвергает. Код это уже делает верно (`infobase_extension_set_active` шлёт `update --active`),
   ошибка была только в этой записи.
4. Дать `init --dry-run` различать «создана» и «уже была» для серверной ИБ — способ найден и
   заведён задачей #100: `ibcmd config generation-id` ничего не создаёт и отвечает нулём только
   когда база существует и читается этими учётными данными (замерено на 8.3.27.2074). Остаётся
   один замер: что она отвечает на живой СУБД-базе; argv-паритет с `create` уже проверен.
5. Переименовать `Export*`-типы домена `infobase_export` в нейтральные к направлению: модуль
   описывает перенос ИБ в обе стороны, а `infobase restore` переиспользует `ExportIntent`,
   `ExportProvider` и `ExportTargetState`. Переименование не меняет провод (значения на нём
   уже нейтральны), но затрагивает много файлов, поэтому идёт отдельной задачей.

6. Довести квитанцию о провайдере до всех операций (`DEC.2026-09-14.RECEIPT-EXPLAINS-PROVIDER-CHOICE`):
   матрица, ключ `providers.<операция>`, три правила валидации и снятие `builder` сделаны;
   квитанция `provider` (`selected`/`origin`/`skipped`) есть только у экспортного семейства,
   где готовность пробуется по цепочке. `init`, `build`, `load`, `dump`, `extensions`,
   `syntax`, `make` берут первого из плана без пробы и квитанции не печатают — им нужна
   единая проверка готовности до запуска и поле `provider` в форме ответа (версия формы).
7. Реализовать агентский провайдер, шаг 2 (`DEC.2026-09-14.AGENT-*`): провайдер `agent` для файловой и кластерной базы —
   `tools.designer_agent` по образцу `tools.edt_cli`, SSH-клиент в процессе без pty, сессия
   на время workspace lock, типы ответа с закрытым `error-type`, страж признаёт агентский JSON
   структурным. Первыми через сессию идут `generation-id` (#99), `dump`, `build`.
9. Реализовать решения о цели и публикации: `DEC.2026-09-14.TARGET-KIND-IS-DECLARED-NOT-PARSED`,
   `TARGET-HAS-TWO-ADDRESSES`, `RUNNER-PUBLISHES-TO-A-WEB-SERVER`, `LAUNCH-OPENS-THE-PUBLISHED-BASE`,
   `PUBLICATION-IS-NEVER-A-DEFAULT-STEP`. Идёт после шага 1 матрицы провайдеров.
10. Реализовать цель «автономный сервер», шаг 3: вид цели «автономный сервер» — `infobase.connection: ws=…`,
   секция `infobase.standalone`, провайдер `agent` через шлюз `ibsrv`, `ibcmd --pid` только
   для чтения после прогрева; при запуске `ibsrv` раннером — без extended-флага и с
   `--ssh-host-key`.

11. Написать фальсификаторы для девяти правил, чьё поведение уже есть в коде, а теста нет:
   `INV.CLI.NESTED-ORCHESTRATION-DOES-NOT-RELOCK`, `INV.CLI.SIDECAR-FAILURE-DOES-NOT-RELEASE-THE-LOCK`,
   `INV.CLI.PREVIEW-RETURNS-AFTER-TOOL-LOOKUP`, `INV.CONFIG.UNSAFE-COMBINATIONS-ARE-REJECTED-BEFORE-DISPATCH`,
   `INV.MCP.ADMISSION-IS-SHARED-BY-BOTH-TRANSPORTS`, `INV.USE-CASES.A-STALE-PROBE-LOG-IS-REMOVED-FIRST`,
   `INV.USE-CASES.BLOCKS-EXCHANGE-TYPED-CONTEXT-ONLY`, `INV.USE-CASES.STAGING-SHARES-THE-PARENT-DIRECTORY`,
   `INV.CLI.PREVIEW-LEAVES-A-LOG-ENTRY`. Остальные одиннадцать `planned` ждут не теста, а кода:
   их решения сами `planned`. Список даёт `spec/arch/index.md` по колонке «проверяется».
12. Починить пробел, найденный при написании тестов: `dump --dry-run` создаёт файл журнала
   действий пустым, хотя превью обязано оставлять след вызова
   (`INV.CLI.PREVIEW-LEAVES-A-LOG-ENTRY`). Сначала правка поведения, потом фальсификатор.

13. Реализовать `DEC.2026-09-14.THE-TARGET-MAY-LIVE-ON-ANOTHER-MACHINE` вместе с агентским
   провайдером: локальность `workPath`, разрешение путей на стороне цели, объявленный канал
   обмена и отказ managed-режима при удалённой точке входа. Четыре правила ждут кода.

14. ([#114](https://github.com/IngvarConsulting/v8-runner-rust/issues/114)) Довести живую проверку форм `data` до четырёх команд, у которых её пока нет: `bootstrap`,
   `config init`, `tools download`, `test`. Первой нужна настоящая база, второй — отдельный
   рабочий каталог в харнессе, третьей — сеть, четвёртой — платформа с YaXUnit. Сейчас их
   формы держит только сверка схемы с типом (`generated_command_data_schemas_are_current`):
   переименованное поле она поймает, а расхождение обещания с живым ответом — нет.

15. ([#115](https://github.com/IngvarConsulting/v8-runner-rust/issues/115)) Закрыть состав полей у вариантов размеченного перечисления в порождённых формах: schemars
   выносит тег варианта в ссылающийся объект, поэтому определения вроде `ModuleIssue`
   остаются открытыми и добавленное в них поле проверку не валит. Затрагивает `syntax`,
   `test` и `make` — везде, где в ответе есть размеченный перечень.

16. ([#116](https://github.com/IngvarConsulting/v8-runner-rust/issues/116)) Свести подпись узла и его знак в одном месте. Сейчас подпись собирает каждый
   рендерер сам («completed with warnings»), а знак выводится из подробностей — при
   расхождении правится подпись, и найдено это было тестами, а не правилом. Правило
   «подпись узла согласована с его знаком» пока не заведено: у него нет фальсификатора,
   который не переписывал бы все четырнадцать рендереров.

## Rules

- Keep this file short and active-only.
- Move closed task detail into `spec/archive/`.
- If a task changes a public or architectural contract, update the ADR and active docs layer
  before implementation.
- Promote only immediately executable work here; keep broader ADR reconciliation in
  `ADR_DERIVED_BACKLOG.md`.

## Historical Records

- [spec/archive/IMPLEMENTATION_TODO_2026-04-30.md](archive/IMPLEMENTATION_TODO_2026-04-30.md):
  closed task ledger moved out of the active file.
- [spec/archive/MCP_IMPLEMENTATION_PLAN_2026-03-21.md](archive/MCP_IMPLEMENTATION_PLAN_2026-03-21.md):
  closed MCP rollout history.
- [spec/archive/completed-tasks-t22.md](archive/completed-tasks-t22.md):
  closed universal tool extension preparation task.
- [spec/archive/completed-tasks-t21.md](archive/completed-tasks-t21.md):
  closed local config overlay task.
- [spec/archive/completed-tasks-t23.md](archive/completed-tasks-t23.md):
  closed YAML schema support for config editing.
- [spec/archive/completed-tasks-t24.md](archive/completed-tasks-t24.md):
  closed source-backed tool extension change-detection task.
- [spec/archive/completed-tasks-t25.md](archive/completed-tasks-t25.md):
  closed JSON Schema descriptions and config alias removal task.
- [spec/archive/completed-tasks-t26.md](archive/completed-tasks-t26.md):
  closed post-T25 public `basePath` removal and schema URL follow-up.

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

7. Реализовать агентский провайдер, шаг 2 (`DEC.2026-09-14.AGENT-*`). Сделано: `tools.designer_agent`
   (managed/attached), встроенный SSH-клиент (`russh`) без pty, типы ответа с закрытым `error-type`,
   `dump` через агента (экспериментально, по ключу). Осталось: `build` через агента одной
   сессией на время workspace lock (`DEC.2026-09-14.AGENT-SESSION-LIVES-WITH-THE-LOCK`),
   `generation-id` (#99), перевод строки `dump` из experimental после живого прогона через
   раннер, страж «агентский JSON — структурный вывод».
8. Реализовать долгоживущего агента (`DEC.2026-09-15.A-KEPT-AGENT-LIVES-WITH-THE-WORKSPACE`):
   `tools.designer_agent.lifetime: command | workspace`, удостоверение `<workPath>/agent/agent.json`,
   проверка перед использованием (pid, личность, аутентификация), свободный порт на старте,
   команды `agent start | stop | status`, остановка своего агента перед другим исполнителем и
   при завершении MCP-сервера; фальсификаторы для трёх новых правил. Ожидаемая экономия — 6–9 с
   на команду (замер УТ 15.09.2026).
10. Реализовать цель «автономный сервер», шаг 3: вид цели «автономный сервер» — `infobase.connection: ws=…`,
   секция `infobase.standalone`, провайдер `agent` через шлюз `ibsrv`, `ibcmd --pid` только
   для чтения после прогрева; при запуске `ibsrv` раннером — без extended-флага и с
   `--ssh-host-key`.

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

# Active TODO For `v8-runner`

This file tracks open implementation work only.

## Current Status

- Open tasks as of `2026-09-11`: 5.

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
   `--security-profile-name`, `--used-in-distributed-infobase` и `--scope`; `--active` вынесен
   отдельной подкомандой `activate`.
4. Дать `init --dry-run` различать «создана» и «уже была» для серверной ИБ — способ найден и
   заведён задачей #100: `ibcmd config generation-id` ничего не создаёт и отвечает нулём только
   когда база существует и читается этими учётными данными (замерено на 8.3.27.2074). Остаётся
   один замер: что она отвечает на живой СУБД-базе; argv-паритет с `create` уже проверен.
5. Переименовать `Export*`-типы домена `infobase_export` в нейтральные к направлению: модуль
   описывает перенос ИБ в обе стороны, а `infobase restore` переиспользует `ExportIntent`,
   `ExportProvider` и `ExportTargetState`. Переименование не меняет провод (значения на нём
   уже нейтральны), но затрагивает много файлов, поэтому идёт отдельной задачей.

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

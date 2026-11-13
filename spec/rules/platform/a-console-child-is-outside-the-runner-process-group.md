---
id: INV.PLATFORM.A-CONSOLE-CHILD-IS-OUTSIDE-THE-RUNNER-PROCESS-GROUP
check:
  - tests/cli_infobase_cross_platform.rs::a_ctrl_break_to_the_runner_group_leaves_its_console_child_to_the_runner
  - src/platform/process.rs::a_ctrl_break_to_the_runner_group_misses_its_managed_child
  - tests/architecture_guardrails.rs::a_platform_spawn_leaves_the_runner_process_group
---

# Консольный потомок раннера живёт вне его группы процессов

На Windows раннер запускает своих потомков в их собственной группе процессов консоли
(`CREATE_NEW_PROCESS_GROUP`). Так идут консольные утилиты — `ibcmd`, `rac`, `1cedtcli` и его
интерактивные сеансы, `git`, `powershell`, `taskkill` — и потомки в job-объекте; клиентам
платформы с окном (`1cv8`, `1cv8c`) флаг ничего не меняет. Событие консоли, посланное группе
раннера, — `GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid раннера)` у того, кто запустил
раннер в своей группе, — получает только раннер. Потомок такого события не получает и не
обрывается им сам: его снимает раннер по своим правилам отмены, а критическая фаза, например
запись базы, доводится до конца с `deferred: true`. Снятие потомка — job-объект, `taskkill` и
`TerminateProcess` — групп не касается.

Вне правила остаются клиент, которого держит Unica (`V8_RUNNER_CLIENT_OWNER`): он намеренно
остаётся в группе и job-объекте раннера
([INV.PLATFORM.HOST-OWNED-CLIENT](host-owned-client.md)), — службы стенда со своей консолью
([INV.PLATFORM.A-SERVICE-OUTLIVES-THE-RUNNER](a-service-outlives-the-runner.md)) и отделённый
запуск браузера. Не покрыто и то, что консоль доставляет всем своим процессам, не глядя на
группы: `Ctrl+Break` с клавиатуры и закрытие окна. `Ctrl+C` в группе потомка выключен самим
флагом, поэтому `Ctrl+C` с клавиатуры потомка тоже не обрывает.

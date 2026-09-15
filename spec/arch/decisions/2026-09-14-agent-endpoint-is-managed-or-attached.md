---
id: DEC.2026-09-14.AGENT-ENDPOINT-IS-MANAGED-OR-ATTACHED
status: active
governs: product
realized: [tests/cli_dump_agent.rs::managed_agent_dumps_through_the_built_in_ssh_client_and_reads_the_result_from_disk, tests/cli_dump_agent.rs::an_unreachable_attached_agent_is_refused_and_no_process_is_launched_instead]
supersedes: []
superseded-by: null
establishes: [INV.PLATFORM.LOCAL-AGENT-READS-RESULTS-FROM-DISK, INV.PLATFORM.READINESS-IS-PROBED-NOT-INFERRED, INV.PLATFORM.UNREACHABLE-ATTACHED-IS-A-TYPED-REFUSAL]
---

# У точки входа агента два режима, и режим объявлен

**Решение.** `managed` — раннер поднимает процесс сам (агент Конфигуратора или
автономный сервер), владеет его жизненным циклом и флагами: свои для своего пути
плюс пользовательские из конфига, пользовательские приоритетнее, конфликт —
ошибка валидации. `attached` — раннер подключается к процессу, поднятому без него,
не добавляет и не запрещает флагов, не перезапускает его и не поднимает свой рядом.
Режим выводится из конфига: `launch` или `auto-start` против `gate` или `attach`.
Недоступный `attached` — типизированный отказ `endpoint_unreachable`, а не переход
в `managed`.

**Почему.** Раннер должен уметь и то и другое: поднять агент для сборки и
подключиться к серверу, который пользователь держит для отладки веб-клиента. Но
переключение на лету означало бы второй процесс на той же базе — конфликт
блокировок.

**Не затрагивает.** Параметры запуска чужого процесса: как и с какими флагами
поднят автономный сервер, решает пользователь.

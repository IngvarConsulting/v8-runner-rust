---
id: INV.USE-CASES.EXTENSIONS-LIST-TAKES-THE-LIVE-AGENT
check:
  - tests/cli_live_agent_portable.rs::extensions_list_takes_the_live_agent_and_reports_no_prefix
---

# `extensions list` при живом агенте берёт список у него

Если живой агент рабочей копии уже поднят, `extensions list` берёт список у него. Второй
процесс платформы не запускается. Это действует, когда цепочка `extensions` — цепочка
умолчаний, в которой агент не первый (файловая база). Квитанция называет агента, а исполнителей
цепочки до него — в `skipped` с причиной. `name_prefix` у записей — `null`: агент применённый
префикс не подтверждает. Остальные поля те же, что у ответа `ibcmd` той же базы. Без живого
агента и при ключе `providers.extensions` порядок цепочки прежний.

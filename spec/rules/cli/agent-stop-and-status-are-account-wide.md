---
id: INV.CLI.AGENT-STOP-AND-STATUS-ARE-ACCOUNT-WIDE
check:
  - tests/cli_live_agent_portable.rs::agent_stop_removes_the_live_agent_and_status_lists_it
  - tests/architecture_guardrails.rs::every_scenario_is_dispatched_under_the_workspace_lock
---

# `agent stop` и `agent status` работают в области учётной записи ОС

Живые агенты принадлежат учётной записи ОС, а не рабочей копии. Поэтому `agent stop` и
`agent status` не берут замок `workPath` и отвечают, пока в рабочей копии идёт другая
команда.

- Без `--all` команда работает с живым агентом цели проекта. Проект ей нужен только для
  того, чтобы найти ключ агента; рабочий каталог она не готовит. В ключ входит версия
  платформы, поэтому без найденной платформы проекта команда отказывает, как отказала бы
  команда проекта, а с `--all` платформа не нужна.
- С `--all` команда работает со всеми живыми агентами учётной записи ОС и проекта не требует:
  без `v8project.yaml` она не отказывает.

`agent hold` и `agent release` по-прежнему берут замок `workPath`.

---
id: INV.CLI.AN-AGENT-HOLD-IS-IDEMPOTENT-PER-HOLDER
check:
  - tests/cli_live_agent_portable.rs::agent_hold_is_idempotent_and_release_removes_the_mark
  - tests/agent_keeper.rs::a_repeated_ttl_hold_keeps_the_later_term
---

# Повторное удержание тем же держателем ничего не добавляет

`v8-runner agent hold` ставит держателя на живого агента цели проекта (`--config`). Если
живого агента нет, команда его поднимает. Отметки разных держателей складываются. Повторный
`hold` того же держателя на того же агента второй отметки не ставит: у срока он продлевает
срок, у процесса ничего не меняет. `release` снимает отметку держателя с агента своей цели.
`release` без отметки ничего не меняет и не отказывает.

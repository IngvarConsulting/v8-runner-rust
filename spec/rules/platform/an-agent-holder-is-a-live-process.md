---
id: INV.PLATFORM.AN-AGENT-HOLDER-IS-A-LIVE-PROCESS
check:
  - tests/agent_keeper.rs::the_agent_closes_when_its_holder_process_exits
  - tests/agent_keeper.rs::a_dead_process_cannot_hold_the_agent
  - tests/agent_keeper.rs::a_ttl_holder_expires_and_takes_the_agent_with_it
  - tests/cli_live_agent_portable.rs::agent_hold_refuses_a_holder_that_is_not_that_live_process
  - src/support/machine.rs::a_process_of_another_account_is_told_apart
---

# Держатель агента — живой процесс или срок

Держатель живого агента бывает такого вида:

- **процесс** (`--holder-pid`, необязательный `--holder-started-at`). Время старта
  процесса раннер читает у ОС в момент `hold`. Если время старта передано (UTC в формате
  RFC 3339), раннер сверяет его с прочитанным с точностью до секунды и при расхождении
  отказывает. Тики FILETIME в Windows и jiffies в Linux точнее секунды, поэтому точное
  сравнение отказывало бы ложно. Держатель пропал, если процесса с этим
  pid нет или у него другое время старта. Так повторно выданный pid не продлевает чужое
  удержание. Процесс другой учётной записи ОС держателем быть не может;
- **срок** (`--ttl <секунды>`) — для держателя без долгоживущего процесса. Держатель
  пропадает, если повторный `hold` не продлил срок до его истечения. Срок считается по
  часам реального времени, так что сон машины его не продлевает.

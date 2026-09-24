---
id: INV.USE-CASES.AN-UNCHANGED-GENERATION-IS-NOT-DUMPED
check: [tests/cli_dump_agent.rs::an_unchanged_generation_is_not_dumped_twice]
---

# Неизменившееся поколение не выгружается

Полная или инкрементальная выгрузка через агента, увидев поколение, равное записанному
после последней удачной загрузки или выгрузки, отвечает успехом с сообщением и не
отправляет агенту команду выгрузки.

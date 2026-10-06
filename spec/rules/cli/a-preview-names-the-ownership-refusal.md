---
id: INV.CLI.A-PREVIEW-NAMES-THE-OWNERSHIP-REFUSAL
check:
  - tests/cli_infobase_owner.rs::a_preview_names_the_ownership_refusal_and_writes_nothing
---

# Превью называет отказ по владельцу

Превью команды записи на базе другой рабочей копии отказывает так же, как отказал бы сам
прогон. Команды записи и чтения определены в
`INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).

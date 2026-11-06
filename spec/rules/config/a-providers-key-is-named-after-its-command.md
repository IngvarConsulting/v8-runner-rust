---
id: INV.CONFIG.A-PROVIDERS-KEY-IS-NAMED-AFTER-ITS-COMMAND
check:
  - src/config/schema.rs::generated_schema_artifacts_are_current
  - src/config/schema.rs::provider_keys_match_the_command_vocabulary
  - src/config/schema.rs::provider_keys_without_a_config_entry_are_refused_at_load
---

# Ключ выбора исполнителя назван именем команды

Опубликованная схема знает ровно тринадцать действующих ключей `providers.*`, и каждый совпадает с
именем команды словаря: `push`, `pull`, `apply`, `reset`, `upload`, `download`, `diff`,
`make`, `convert`, `extensions`, `infobase.create`, `infobase.dump`, `infobase.restore`;
`providers.publish`, `providers.check` и `providers.sessions` отклоняются.

Прежние имена ключей, принятые скрытыми синонимами на один цикл выпуска
(`INV.CONFIG.A-KEY-SYNONYM-IS-MARKED-DEPRECATED-IN-THE-SCHEMA`), в этот перечень не входят:
они живут в схеме сверх него с пометкой `deprecated`.

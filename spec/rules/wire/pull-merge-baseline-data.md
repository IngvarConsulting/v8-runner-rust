---
id: CTR.WIRE.PULL-MERGE-BASELINE-DATA
version: 1
artifact: docs/schemas/command-data/pull-merge-baseline.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/cli_external_full_pull.rs::accepting_a_merge_baseline_preserves_outgoing_changes_and_refuses_a_moved_base
  - tests/cli_external_full_pull.rs::a_pruned_accepted_merge_is_confirmed_without_rewriting_its_memory
  - tests/cli_external_full_pull.rs::an_expired_unaccepted_merge_requires_a_new_fetch_before_platform
---

# `data` при проверке или принятии исходного снимка

`pull --check-merge-baseline` и `pull --accept-merge-baseline` возвращают имя
набора в `source_set`, подтверждённое поколение в `generation` и инструмент
проверки в `provider`. `baseline: ready` означает, что подготовленный снимок
можно принять. `baseline: accepted` означает, что снимок уже принят. Повтор
проверяет, что исходная привязка и поколение ещё действуют.

## Пример

```json
{
  "source_set": "main",
  "generation": "ded76a7b47094542841c173b5a436f0900000000",
  "provider": "ibcmd",
  "baseline": "ready"
}
```

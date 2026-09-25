---
id: INV.USE-CASES.A-TARGET-IS-RECHECKED-BEFORE-PUBLICATION
check:
  - src/use_cases/artifacts.rs::designer_export_rechecks_the_target_before_publishing
  - src/use_cases/dump_config.rs::finalize_edt_dump_revalidates_publish_target_after_import
  - src/use_cases/infobase_export.rs::publication_rejects_target_identity_change_after_provider_execution
  - tests/architecture_guardrails.rs::every_staged_publication_rechecks_its_target_first
---

# Цель перепроверяется перед публикацией

Публикация через промежуточную копию у `make`, `pull`, `download` и `infobase dump` после
работы исполнителя, перед заменой, перепроверяет цель: путь, который за это время стал
указывать в другое место, публикацию останавливает, и цель не трогается.

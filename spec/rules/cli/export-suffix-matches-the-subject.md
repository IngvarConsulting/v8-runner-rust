---
id: INV.CLI.EXPORT-SUFFIX-MATCHES-THE-SUBJECT
check:
  - tests/cli_infobase.rs::invalid_suffix_is_rejected_before_workspace_lock_and_provider_dispatch
  - src/use_cases/infobase_export.rs::configuration_output_suffix_is_derived_from_subject
---

# Расширение файла соответствует предмету выгрузки

Основная конфигурация требует своего расширения файла, расширение конфигурации — своего, снимок базы — своего; несоответствие отклоняется до работы.

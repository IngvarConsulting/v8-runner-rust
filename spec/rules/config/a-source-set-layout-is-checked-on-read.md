---
id: INV.CONFIG.A-SOURCE-SET-LAYOUT-IS-CHECKED-ON-READ
check:
  - src/config/validate.rs::edt_format_rejects_non_external_source_set_without_project_marker
  - src/config/validate.rs::edt_format_rejects_non_external_source_set_without_supported_nature
  - src/config/validate.rs::edt_format_rejects_native_extension_project_when_purpose_mismatches_nature
  - src/config/validate.rs::edt_format_rejects_native_configuration_project_without_manifest
  - src/config/validate.rs::edt_format_accepts_native_configuration_project_layout
  - src/config/validate.rs::edt_external_source_set_rejects_child_project_with_mismatched_kind
  - src/config/validate.rs::designer_external_source_set_rejects_mismatched_top_level_xml_descriptors
  - src/config/validate.rs::designer_external_source_set_accepts_matching_top_level_xml_descriptors
  - src/config/validate.rs::designer_format_allows_missing_source_set_path
  - tests/contract_config_boundary.rs::an_unsupported_combination_is_refused_before_any_utility_runs
---

# Раскладка набора EDT и корня внешних объектов проверяется при чтении конфигурации

Команда, которой нужны исходники, при чтении конфигурации проверяет раскладку наборов: обычный
набор EDT — проект с поддерживаемой природой, совпадающей с объявленным типом, и с
манифестом; корень внешних объектов — проекты EDT или описания Конфигуратора того вида, что
объявлен. Обычный набор формата Конфигуратора при чтении не проверяется: его каталога может
ещё не быть. `download`, `infobase dump`, `infobase restore`, `test --no-push` и
`tools download` исходников не читают и раскладку не проверяют.

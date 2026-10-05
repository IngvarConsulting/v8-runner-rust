---
id: INV.CONFIG.GENERATED-TARGETS-DO-NOT-COLLIDE-WITH-SOURCES
check:
  - src/config/validate.rs::rejects_edt_source_set_path_overlapping_generated_work_target
  - src/config/validate.rs::rejects_reserved_source_set_name
  - src/config/validate.rs::rejects_client_mcp_edt_source_overlapping_generated_export_target
  - src/use_cases/build_project.rs::edt_source_set_named_tool_extensions_and_tool_extension_export_apart
---

# Порождённые каталоги не пересекаются с исходниками

Пути наборов и порождённые рабочие цели не совпадают; зарезервированные имена рабочих каталогов нельзя использовать как имена наборов.

Расширение-инструмент с исходниками EDT выгружается в `workPath/tool-extensions/<имя>`, вне
каталога выгрузок наборов `workPath/designer`: набор с именем `tool-extensions` и
расширение-инструмент не затирают выгрузки друг друга, а исходники расширения-инструмента не
пересекаются с его выгрузкой. Имя `tool-extensions` поэтому не зарезервировано за рабочим
каталогом: выгрузки наборов лежат под `workPath/designer` и с ним не встречаются.

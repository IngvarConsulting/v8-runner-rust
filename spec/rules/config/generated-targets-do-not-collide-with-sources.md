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

Снимок Конфигуратора набора формата EDT лежит под памятью выбранной базы,
`workPath/infobases/<база>/designer/<набор>`, а у набора без памяти базы — внешних обработок
и отчётов и базы с нераспознанным адресом — в `workPath/designer/<набор>`. База при проверке
конфигурации может быть ещё не выбрана, а другая база кладёт снимок под своим ключом,
поэтому исходники набора EDT не пересекаются ни с `workPath/designer/<набор>`, ни со всем
корнем памяти `workPath/infobases`.

Расширение-инструмент с исходниками EDT выгружается в `workPath/tool-extensions/<имя>`, вне
снимков наборов: набор с именем `tool-extensions` и расширение-инструмент не затирают
выгрузки друг друга, а исходники расширения-инструмента не пересекаются с его выгрузкой.
Имя `tool-extensions` поэтому не зарезервировано за рабочим каталогом: снимки наборов лежат
под `workPath/designer` и `workPath/infobases` и с ним не встречаются.

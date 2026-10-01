---
id: INV.USE-CASES.A-TOOL-EXTENSION-IS-NOT-A-PROJECT-SOURCE-SET
check:
  - tests/cli_build.rs::build_text_groups_tool_extension_stages_under_single_build_node
  - src/config/schema.rs::main_schema_and_loader_reject_invalid_tool_extension_shapes
---

# Расширение инструмента не является набором исходников проекта

Расширение, нужное самому инструменту, имеет имя в базе и ровно один источник — исходники
или готовый артефакт; иную форму отвергает схема.

В наборы проекта оно не добавляется, в их порядок не входит и ключом `--source-set` не
адресуется. Готовится оно на стадии сборки, а не запуска клиента. Проектные расширения
остаются наборами.

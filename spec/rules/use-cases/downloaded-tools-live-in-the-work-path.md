---
id: INV.USE-CASES.DOWNLOADED-TOOLS-LIVE-IN-THE-WORK-PATH
check:
  - tests/cli_tools_download.rs::tools_download_sources_writes_source_set_and_local_tool_settings
  - src/change_detection/source_sets.rs::tool_sources_under_the_work_path_are_analyzed_like_any_other
---

# Загрузки `tools download` лежат в `workPath`

`tools download` — пересчитываемый кеш: `.cfe`, EPF Vanessa и исходники onec-client-mcp-devkit
ложатся в `workPath/tools`, рядом с ними — их метки управляемой цели, а пути в
`v8project.local.yaml` указывают туда. Исходники YAxUnit с `--sources` — набор проекта и
по-прежнему распаковываются в `tests`. Прежний `build/tools` раннер не читает и не удаляет.

Исходники расширения-инструмента под `workPath` разбираются чтением файлов, как прежде:
`.gitignore` со строкой `*`, которым `workPath` прячет себя от git, отказа
`source_set_ignored_by_git` не даёт, а каталоги раннера под их корнем не лежат.

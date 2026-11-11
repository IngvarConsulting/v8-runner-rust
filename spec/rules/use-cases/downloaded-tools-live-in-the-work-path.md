---
id: INV.USE-CASES.DOWNLOADED-TOOLS-LIVE-IN-THE-WORK-PATH
check:
  - tests/cli_tools_download.rs::tools_download_sources_writes_source_set_and_local_tool_settings
  - src/change_detection/source_sets.rs::tool_sources_under_the_work_path_are_analyzed_like_any_other
  - tests/cli_tools_download.rs::tools_download_sources_recognizes_the_legacy_tests_marker_in_build
---

# Загрузки `tools download` лежат в `workPath`

`tools download` — пересчитываемый кеш: `.cfe`, EPF Vanessa и исходники onec-client-mcp-devkit
ложатся в `workPath/tools`, рядом с ними — их метки управляемой цели, а пути в
`v8project.local.yaml` указывают туда. Прежний `build/tools` раннер не читает и не удаляет.

Исходники YAxUnit с `--sources` — набор проекта `tests`. Метка, что этот каталог принадлежит
раннеру, — долгоживущее состояние рабочей копии: она пишется в
`localPath/tools/.tests.v8-runner-tools-download.json`, и `localPath` заводится со своим
`.gitignore`. Метка прежних версий `<basePath>/build/.tests.v8-runner-tools-download.json`
признаётся, если новой нет: уже скачанный `tests` повтор без `--force` не отказывает. Её раннер
не пишет и не удаляет, а `build/` не создаёт.

Исходники расширения-инструмента под `workPath` разбираются чтением файлов, как прежде:
`.gitignore` со строкой `*`, которым `workPath` прячет себя от git, отказа
`source_set_ignored_by_git` не даёт, а каталоги раннера под их корнем не лежат.

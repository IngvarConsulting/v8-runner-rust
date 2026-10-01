---
id: INV.USE-CASES.HASH-MEMORY-IS-SCOPED-TO-ITS-BASE-AND-SOURCE
check:
  - src/change_detection/source_sets.rs::base_snapshots_remain_separate_and_reject_a_retargeted_base
  - src/change_detection/source_sets.rs::distinct_non_utf8_source_roots_have_distinct_bindings
  - src/change_detection/source_sets.rs::canonical_source_names_are_not_case_folded_for_memory
  - src/config/loader.rs::spaced_file_parameters_use_the_project_directory_for_runtime_and_memory
  - tests/cli_pull_memory.rs::relative_file_address_uses_the_project_directory_and_matches_absolute_memory
  - src/change_detection/source_sets.rs::ad_hoc_analysis_never_reads_or_writes_memory_and_empty_sources_skip
  - src/change_detection/source_sets.rs::edt_and_external_memory_is_shared_but_designer_identity_ignores_credentials
  - tests/cli_pull_memory.rs::a_pull_from_one_base_does_not_mark_another_base_as_loaded
  - tests/cli_pull_memory.rs::foreign_memory_is_named_in_the_response_without_dispatching_or_exposing_credentials
  - tests/cli_pull_memory.rs::an_ad_hoc_base_never_uses_the_named_hash_baseline
  - src/change_detection/source_sets.rs::standalone_snapshot_uses_gate_address_without_secrets_or_transport_settings
  - src/platform/connection.rs::snapshot_address_identity_ignores_credentials_and_canonicalizes_file_paths
---

# Хеш-память относится к своей базе и исходникам

Хеши загрузки наборов исходников проекта — конфигурации и расширений — лежат в
`workPath/infobases/<имя>/hashes/<набор>.redb`, отдельно для каждой именованной базы.
Адрес без учётных данных, исходный каталог и назначение набора записываются атомарно
со снимком. Адрес в привязке совпадает с адресом, переданным платформе, включая
допустимые пробелы вокруг `=` у `File`. Канонические пути сравниваются по точному
представлению ОС, без потери байтов и сведения регистра: консервативное объединение
имён для замка не является равенством исходников. Исполнитель не участвует в привязке; назначение набора записывается его именем из
YAML (`CONFIGURATION`, `EXTENSION`), адрес сервера — без учёта регистра. Обычный `push`
при чужой памяти отказывает, называя записанную и выбранную привязки и выходы: полный `pull`, если
права база, и `push --full`, если прав каталог; каждый из них заменяет память после
успеха. Старые общие снимки не читаются и не мигрируются: первый `push` после обновления
видит все файлы добавленными.

База по строке соединения не читает и не пишет хеш-память обмена с базой. Общий кеш EDT
и внешних артефактов от базы не зависит. Отсутствующий снимок не объявляется ошибкой
хранилища: первая сборка видит существующие файлы как добавленные, пустой набор пропускается.

Защита от возврата: прежний общий Designer-ключ позволял второй базе взять снимок первой.
Путь и привязку определяет `SourceSetContext`; у контекста без памяти пути к хранилищу
нет вовсе (`storage_path` отвечает `None`), поэтому новый код не может открыть общее
хранилище, не обработав этот случай. Проверки чередуют базы и повторяют ad hoc вызовы.
Хеши расширений-инструментов (`tools.extensions`) пока остаются в общем
`workPath/hash-storages`; их и остальные виды памяти переносит #214.

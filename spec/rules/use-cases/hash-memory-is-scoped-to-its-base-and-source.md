---
id: INV.USE-CASES.HASH-MEMORY-IS-SCOPED-TO-ITS-BASE-AND-SOURCE
check:
  - src/change_detection/source_sets.rs::base_snapshots_remain_separate_and_reject_a_retargeted_base
  - src/change_detection/source_sets.rs::distinct_non_utf8_source_roots_have_distinct_bindings
  - src/change_detection/source_sets.rs::canonical_source_names_are_not_case_folded_for_memory
  - src/config/loader.rs::spaced_file_parameters_use_the_project_directory_for_runtime_and_memory
  - tests/cli_pull_memory.rs::relative_file_address_uses_the_project_directory_and_matches_absolute_memory
  - src/change_detection/source_sets.rs::standalone_snapshot_uses_gate_address_without_secrets_or_transport_settings
  - src/change_detection/source_sets.rs::a_standalone_server_is_remembered_by_its_gate_and_without_it_by_the_direct_gate
  - src/platform/connection.rs::snapshot_address_identity_ignores_credentials_and_canonicalizes_file_paths
---

# Привязка хеш-памяти называет базу и исходники

Привязка хеш-памяти состоит из адреса базы без учётных данных, канонического каталога
исходников, назначения и имени набора; исполнитель в неё не входит. Назначение
записывается именем из YAML (`CONFIGURATION`, `EXTENSION`), имена сервера и базы на
кластере — без учёта регистра. Адрес файловой базы совпадает
с переданным платформе, включая допустимые пробелы вокруг `=` у `File`. Канонический
путь сравнивается по точному представлению ОС, без потери байтов и сведения регистра:
консервативное объединение имён для замка не является равенством исходников.

Адрес автономного сервера — его SSH-шлюз, и строка прямого шлюза рядом с ним адреса не
меняет; без SSH-шлюза адрес — строка прямого шлюза без учётных данных, с именами без учёта
регистра.

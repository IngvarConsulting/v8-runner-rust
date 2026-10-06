---
id: INV.USE-CASES.MEMORY-LIVES-UNDER-THE-BASE-IT-DESCRIBES
check:
  - src/change_detection/source_sets.rs::analysis_state_lies_under_the_work_path_by_logical_context
  - src/change_detection/source_sets.rs::edt_and_external_memory_is_shared_but_designer_identity_ignores_credentials
  - tests/cli_pull_memory.rs::a_pull_from_one_base_does_not_mark_another_base_as_loaded
  - src/change_detection/source_sets.rs::an_edt_designer_copy_lies_under_the_base_memory
  - src/change_detection/source_sets.rs::tool_extension_memory_lies_under_the_base_apart_from_source_sets
  - src/use_cases/version_file.rs::the_copy_lies_under_the_base_and_the_set
  - src/use_cases/agent_session.rs::the_ledger_keeps_one_record_per_source_set_under_the_base
---

# Память лежит под базой, которую описывает

Хеши исходников, поколение базы и файл версий `ConfigDumpInfo.xml` описывают отношение
«один каталог ↔ одна база» и лежат под `workPath/infobases/<имя базы>/`:
`generation.json`, `hashes/` по наборам, `dump-info/<набор>/ConfigDumpInfo.xml`. Запись о
поколении отдельна по предмету — основная конфигурация и каждое расширение — и по
операции; какому инструменту принадлежит токен, держит правило о сравнении токенов.

Под базой лежит только состояние обмена с ней — контекст `designer-<набор>`: его хеши, а у
формата EDT и сам снимок Конфигуратора `designer/<набор>` вместе с его файлом версий. Хеши
исходников расширений-инструментов (`tools.extensions`) лежат там же, в `hashes/tools/`.
Кеш экспорта EDT, контекст `edt-<набор>` и кеш внешних артефактов от базы не зависят и
остаются общими.

Прежняя общая память — `workPath/hash-storages/designer-*.redb`, хеши
расширений-инструментов в `workPath/hash-storages/tool-*.redb`, снимки
`workPath/designer/<набор>` и журнал `workPath/agent/generation/` — не читается, не
переносится и не удаляется: первый `push` после обновления видит все файлы добавленными, а
первая выгрузка агентом не пропускается по поколению.

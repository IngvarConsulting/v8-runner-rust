---
id: INV.USE-CASES.MEMORY-LIVES-UNDER-THE-BASE-IT-DESCRIBES
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/214
---

# Память лежит под базой, которую описывает

Хеши исходников, поколение базы и файл версий `ConfigDumpInfo.xml` описывают отношение
«один каталог ↔ одна база» и лежат под `workPath/infobases/<имя базы>/`:
`generation.json`, `hashes/` по наборам, `dump-info/<набор>/ConfigDumpInfo.xml`. Запись о
поколении отдельна по предмету — основная конфигурация и каждое расширение — и по
операции; какому инструменту принадлежит токен, держит правило о сравнении токенов.

Под базой лежит только состояние обмена с ней — контекст `designer-<набор>`. Кеш экспорта
EDT, контекст `edt-<набор>` и кеш внешних артефактов от базы не зависят и остаются общими.

Проверенный срез — хеши загрузки конфигураций и расширений в
`workPath/infobases/<имя>/hashes/<набор>.redb`, отдельно для каждой именованной базы.
Его держат `src/change_detection/source_sets.rs::analysis_state_lies_under_the_work_path_by_logical_context`,
`src/change_detection/source_sets.rs::edt_and_external_memory_is_shared_but_designer_identity_ignores_credentials`
и `tests/cli_pull_memory.rs::a_pull_from_one_base_does_not_mark_another_base_as_loaded`.
Копия файла версий набора формата Конфигуратора уже лежит в
`workPath/infobases/<имя>/dump-info/<набор>/` — `INV.USE-CASES.A-REPLACED-VERSION-FILE-GIVES-WAY-TO-THE-RUNNER-COPY`.
Файл версий набора формата EDT лежит в общем для всех баз снимке `workPath/designer/<набор>`;
его раскладка по базам, перенос поколения и хешей расширений-инструментов (`tools.extensions`,
пока в общем `workPath/hash-storages`) остаются в #214. Общие старые Designer-хеши
не используются и не мигрируются: первый `push` после обновления видит все файлы
добавленными.

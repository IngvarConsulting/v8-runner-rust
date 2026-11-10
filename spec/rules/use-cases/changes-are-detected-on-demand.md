---
id: INV.USE-CASES.CHANGES-ARE-DETECTED-ON-DEMAND
check:
  - src/change_detection/analyzer.rs::cancellation_during_recoverable_storage_read_is_not_fallback
  - src/use_cases/build_project.rs::cancellation_after_edt_export_keeps_completed_work_in_the_response
  - src/use_cases/build_project.rs::a_completed_full_step_records_hashes_after_deferred_cancellation
  - src/use_cases/build_project.rs::changed_extension_only_loads_extension_and_preserves_other_storage
  - src/use_cases/build_project.rs::source_set_build_analyzes_and_loads_only_requested_source_set
  - src/change_detection/source_sets.rs::analysis_state_lies_under_the_work_path_by_logical_context
  - src/change_detection/scanner.rs::every_selected_file_is_hashed_regardless_of_mtime
  - src/change_detection/scanner.rs::a_restored_copy_with_an_old_mtime_is_hashed
  - src/change_detection/scanner.rs::changed_bytes_with_the_exact_remembered_old_mtime_are_hashed
  - src/change_detection/scanner.rs::a_file_is_hashed_as_git_hashes_it_without_filters
  - src/change_detection/analyzer.rs::changed_bytes_with_the_exact_remembered_old_mtime_are_a_change
  - src/change_detection/analyzer.rs::interrupted_analysis_keeps_memory_and_never_falls_back
  - src/change_detection/scanner.rs::partial_reads_fail_without_a_hash_and_cancel_before_the_next_read
  - src/change_detection/scanner.rs::an_interrupted_scan_returns_no_partial_snapshot
  - src/change_detection/scanner.rs::hashing_streams_files_larger_than_its_working_buffer
  - src/change_detection/scanner.rs::a_parallel_read_gives_the_snapshot_of_a_sequential_one
  - src/change_detection/scanner.rs::a_failed_read_stops_the_other_workers_without_a_snapshot
  - src/change_detection/scanner.rs::a_cancellation_stops_the_workers_without_a_snapshot
  - src/change_detection/scanner.rs::the_earliest_failing_file_is_reported_whichever_fails_first
  - src/change_detection/analyzer.rs::a_module_restored_with_an_old_mtime_is_a_change
  - src/change_detection/analyzer.rs::a_file_rewritten_with_the_same_content_is_not_a_change
  - src/change_detection/source_sets.rs::an_ad_hoc_base_is_remembered_by_its_address_and_empty_sources_skip
  - src/change_detection/scanner.rs::service_and_generated_paths_are_never_scanned
  - src/use_cases/source_changes.rs::git_names_an_edit_an_addition_a_removal_and_a_rename
  - src/use_cases/source_changes.rs::an_edit_of_the_same_size_and_time_is_read_again_when_git_cannot_trust_the_time
  - src/use_cases/source_changes.rs::a_line_ending_only_edit_is_not_a_change
  - src/use_cases/source_changes.rs::assume_unchanged_and_skip_worktree_files_are_read_directly
  - src/use_cases/source_changes.rs::a_set_outside_git_is_read_file_by_file
  - src/use_cases/source_changes.rs::a_failing_git_falls_back_to_reading_files
  - src/use_cases/source_changes.rs::a_set_in_a_submodule_is_read_file_by_file
  - src/use_cases/source_changes.rs::a_nested_repository_inside_the_set_is_read_file_by_file
  - src/use_cases/source_changes.rs::git_and_scan_snapshots_agree
  - src/platform/git.rs::a_cancelled_git_query_ends_git_and_answers_interrupted
  - src/platform/git.rs::the_worktree_answer_is_parsed_from_ls_files_and_status
  - src/platform/git.rs::a_cancelled_status_leaves_no_index_lock
  - src/platform/git.rs::the_id_is_the_one_git_add_would_store
  - src/use_cases/source_changes.rs::the_memory_after_a_push_holds_what_git_add_would_store
  - src/use_cases/source_changes.rs::a_memory_of_the_bytes_is_compared_by_its_own_hasher_under_git
  - tests/architecture_guardrails.rs::change_detection_has_no_background_watcher
  - tests/architecture_guardrails.rs::change_detection_never_reads_the_executor_choice
---

# Изменения ищет та команда, которой нужен ответ

Фонового наблюдателя нет: анализ изменений запускает та команда, которой нужно решение о
сборке, экспорте или загрузке. Состояние анализа лежит под `workPath` по логическому
контексту набора.

Отсутствующий снимок не объявляется ошибкой хранилища: первая сборка видит существующие
файлы как добавленные, пустой набор пропускается.

Память хранит хеш blob git каждого файла и, один раз на всю память, каким хешером он взят
(#575): под git — тот хеш, что сохранил бы `git add`, без git — хеш байтов файла, как
`git hash-object --no-filters`. Изменение — файл, хеш которого отличается от хеша в памяти,
новый файл и удалённый. Хеши разных хешеров между собой не сравниваются: память хешей байтов
под git сравнивается своим хешером, чтением файлов (причина `bytes_memory`), пока полная
выгрузка или отправка не запишет её хешами git; непустая память хешей git без git даёт
полную загрузку.

**Набор в рабочей копии git.** Набор формата `DESIGNER`, который лежит в рабочей копии git, не
в подмодуле и не под `workPath`, берёт изменения у git: раннер спрашивает один раз на рабочую
копию `git status --porcelain=v2` по корням наборов и `git ls-files -s` за хешами индекса. У
файла, который git называет неизменным, хеш — из индекса; файл, который git называет
изменённым или неотслеживаемым, файл с `assume-unchanged` или `skip-worktree` и файл
незавершённого слияния хеширует сам git во временном индексе, куда раннер кладёт записи
настоящего индекса этих путей: преобразование концов строк видит то, что лежит в индексе,
как у `git add`. Ни объектов в хранилище, ни настоящего индекса этот шаг не пишет. Каталоги,
которые обход пропускает,
`workPath` внутри корня и ссылки пропускаются и здесь, тем же предикатом. Fsmonitor и кэш
неотслеживаемых раннер не включает: работает то, что настроено в репозитории, и ответ это
называет. Раннер передаёт git `-c core.checkStat=default -c core.trustctime=true`.

Под git метаданные заменяют чтение содержимого так же, как у самого git: файл, который git
по размеру и времени считает неизменным, раннер не читает. Это решение владельца, и оно
ослабляет гарантию #447 для набора под git: правка того же размера с подставленным старым
временем, которое раньше записи индекса, не видна. Время, которому git верить не может, — не
раньше записи индекса — git проверяет чтением, и такая правка видна. Правка, которая меняет
только концы строк при `core.autocrlf` или атрибутах `eol`, изменением не считается.

**Набор без git.** Вне рабочей копии git (и когда git не найден), в подмодуле, при вложенном
репозитории внутри набора, в репозитории хешей SHA-256 и при отказе git изменения
подтверждаются содержимым: каждый выбранный обычный файл
хешируется при анализе, как `git hash-object --no-filters`, включая файл с прежним размером и
точно запомненным старым временем изменения. Перенос, восстановление или новая отметка
времени сами по себе не означают изменения: одинаковые байты не вызывают загрузку.
Метаданные не заменяют чтение содержимого. Хеширование идёт потоком с ограниченным рабочим
буфером без ограничения размера файла. Набор с достаточным числом файлов читается в несколько
потоков; снимок и решение от числа потоков и порядка их завершения не зависят, записи снимка
идут в порядке обхода. Отказ git не отказывает команде: набор читается сам, а память,
записанная под git, при этом даёт полную загрузку. Исходники формата
EDT читаются всегда: в базу идёт их копия под `workPath`.

Ответ `status` и `push` называет путь каждого разобранного набора
(`CTR.WIRE.STATUS-DATA`, `CTR.WIRE.PUSH-DATA`).

Анализ решения перед загрузкой замечает отмену между файлами и до и после чтения
очередного блока. Это кооперативная проверка, не гарантия срока прерывания блокирующего
чтения. Отмена во время вопроса к git завершает git; `git status`, которому разрешено
обновить индекс, раннер при отмене доводит до конца: принудительно завершённый, он оставил бы
`.git/index.lock`. Прерванный анализ не отдаёт частичный
снимок, не пишет память и не превращается в запасную полную загрузку или чтение файлов.
Признак отмены спрашивает поток, начавший анализ; отмена
останавливает потоки между файлами и между блоками, ошибка чтения — потоки на более поздних
файлах, и сообщается самый ранний по обходу отказавший файл. Отмену команды оформляет её существующий владелец
(`INV.WIRE.A-CANCELLATION-ANSWERS-AS-A-CANCELLATION`). После завершённого успешного шага или
публикации память результата записывается и при отложенной отмене.

Служебные и
порождённые каталоги пропускаются только внутри набора; корень набора сканируется всегда
(`INV.USE-CASES.A-SELECTED-ROOT-IS-SCANNED-WHATEVER-ITS-NAME`). Анализ отвечает только на вопрос, что делать, — выбора исполнителя
он не касается.

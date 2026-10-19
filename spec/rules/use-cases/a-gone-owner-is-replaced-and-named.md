---
id: INV.USE-CASES.A-GONE-OWNER-IS-REPLACED-AND-NAMED
check:
  - tests/cli_infobase_owner.rs::a_gone_owner_is_replaced_and_named
  - tests/cli_infobase_owner.rs::a_project_copied_whole_leaves_no_live_owner
  - tests/cli_infobase_owner.rs::an_unreadable_local_layer_of_the_owner_keeps_it_alive
  - tests/cli_infobase_owner.rs::a_secret_in_the_unparsable_layer_of_the_owner_never_reaches_the_answer
  - tests/cli_infobase_owner.rs::a_fifo_in_place_of_the_owner_project_file_does_not_hang_the_command
  - src/use_cases/infobase_owner.rs::a_record_of_this_project_and_host_on_another_machine_is_named_as_maybe_this_one
  - tests/cli_infobase_owner.rs::an_owner_on_another_machine_is_never_replaced
  - src/use_cases/infobase_owner.rs::a_host_rename_keeps_the_machine
  - src/use_cases/infobase_owner.rs::an_owner_whose_config_pair_changes_is_kept_alive_until_a_coherent_read
  - src/config/loader.rs::declared_infobases_refuse_two_absences_that_never_existed_together
  - src/config/loader.rs::declared_infobases_refuse_an_ancestor_exchange_without_a_named_declaration_gap
  - src/config/loader.rs::declared_infobases_keep_a_stable_pair_during_unrelated_ancestor_child_churn
  - src/config/loader.rs::config_route_notifications_require_a_confirmed_native_filesystem
  - src/config/loader.rs::config_route_move_witness_stays_invalid_after_rechecking
  - src/config/loader.rs::declared_infobases_keep_a_stable_pair_with_both_declarations
  - src/config/loader.rs::declared_infobases_hold_the_windows_route_and_captured_files_against_mutation
---

# Ушедшего владельца сменяют и называют

На базе, названной в местном слое, владельца, чей каталог на этой машине исчез или больше не
объявляет базу, команда записи сменяет сама, и ответ команды командной строки об этом говорит; инструменту
MCP это обещает `INV.MCP.A-BOUNDARY-NOTE-REACHES-THE-TOOL-ANSWER`. Живого владельца
раннер не трогает: копию с другой машины, как и копию этой машины, чей согласованный состав объявлений
в проектном и местном файлах нельзя установить, раннер считает живой, и запись в её базу идёт с предупреждением
(`INV.USE-CASES.A-WRITE-ON-A-BASE-OF-ANOTHER-COPY-RUNS-WITH-A-WARNING`). Машину называет её идентификатор, который
переживает смену имени хоста, а объявленный путь базы сравнивается канонически от каталога
владельца — поэтому проект, скопированный целиком вместе с меткой, прежнего владельца живым не
оставляет. Команды записи и чтения определены в
`INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).

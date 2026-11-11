---
id: INV.USE-CASES.A-PULL-BY-CHANGES-RECORDS-WHAT-IT-WROTE
check:
  - tests/cli_pull_memory.rs::a_push_after_a_pull_by_changes_loads_nothing
  - tests/cli_pull_memory.rs::a_user_edit_after_a_pull_by_changes_is_still_loaded
  - tests/cli_pull_memory.rs::a_file_removed_by_a_pull_by_changes_is_forgotten
  - tests/cli_pull_memory.rs::a_failed_pull_by_changes_leaves_the_memory
  - src/change_detection/analyzer.rs::a_dump_remembers_what_it_wrote_and_keeps_an_earlier_user_edit
  - src/change_detection/analyzer.rs::an_edit_after_the_dump_ended_stays_a_change
  - src/change_detection/analyzer.rs::a_commit_after_one_changed_file_touches_only_its_rows
  - src/change_detection/scanner.rs::a_file_whose_time_moved_while_it_was_read_is_left_out
  - src/use_cases/dump_config.rs::the_command_keeps_an_edit_after_the_dump_as_a_change
  - src/change_detection/analyzer.rs::a_set_without_memory_of_the_base_is_not_watched
  - tests/cli_agent_standalone.rs::the_memory_forgets_the_files_an_sftp_dump_removed
  - src/use_cases/source_changes.rs::a_pull_by_changes_under_git_remembers_what_it_wrote_by_git_ids
---

# Выгрузка по изменившемуся запоминает то, что записала

`pull` по изменившемуся в формате `DESIGNER` — у Конфигуратора, `ibcmd` и агента — после
удачной выгрузки над базой, которую во время неё не правили (поколение до и после одно),
записывает в память набора хеши файлов, которые выгрузка записала, и забывает файлы, которые
она удалила. Остальная память не меняется. Поэтому `push` сразу после такого `pull` ничего не
грузит, а правка человека, сделанная до выгрузки, остаётся изменением.

Хеш записанного файла берёт тот хешер, которым записана память: у памяти git — тот, что
сохранил бы `git add` (git во временном индексе рабочей копии), у памяти хешей байтов — раннер,
как `git hash-object --no-filters`; память git без git ничего не запоминает
(`INV.USE-CASES.CHANGES-ARE-DETECTED-ON-DEMAND`).

Записанным считается файл, время изменения которого не раньше начала выгрузки и не позже её
конца; начало и конец берутся по часам файловой системы набора. Файл, изменённый после конца
выгрузки, и файл, время которого сдвинулось, пока его хешировали, остаются изменением: их правили
уже после платформы. Правка в тот же тик часов файловой системы, что и отметка конца, считается
записью платформы: на Windows этот тик около 15 мс. Правку, сделанную во время выгрузки, от записи платформы так не отличить, и
она запоминается; файл, удалённый после конца выгрузки, тоже забывается как удалённый ею — это
остаётся задачей #170.

Отказ или отмена выгрузки, правка базы во время неё, неизвестное поколение и отсутствие памяти
о базе память не меняют: следующий `push` грузит записанное выгрузкой. Память полной выгрузки
описывает `INV.USE-CASES.FULL-PULL-RECORDS-THE-PUBLISHED-TREE`, содержимое как основание
решения — `INV.USE-CASES.CHANGES-ARE-DETECTED-ON-DEMAND`.

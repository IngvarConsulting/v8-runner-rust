---
id: INV.USE-CASES.A-PULL-BY-CHANGES-RECORDS-WHAT-IT-WROTE
check:
  - tests/cli_pull_memory.rs::a_push_after_a_pull_by_changes_loads_nothing
  - tests/cli_pull_memory.rs::a_user_edit_after_a_pull_by_changes_is_still_loaded
  - tests/cli_pull_memory.rs::a_file_removed_by_a_pull_by_changes_is_forgotten
  - tests/cli_pull_memory.rs::a_failed_pull_by_changes_leaves_the_memory
  - src/change_detection/analyzer.rs::a_dump_remembers_what_it_wrote_and_keeps_an_earlier_user_edit
  - src/change_detection/analyzer.rs::a_set_without_memory_of_the_base_is_not_watched
---

# Выгрузка по изменившемуся запоминает то, что записала

`pull` по изменившемуся в формате `DESIGNER` — у Конфигуратора, `ibcmd` и агента — после
удачной выгрузки над базой, которую во время неё не правили (поколение до и после одно),
записывает в память набора хеши файлов, которые выгрузка записала, и забывает файлы, которые
она удалила. Остальная память не меняется. Поэтому `push` сразу после такого `pull` ничего не
грузит, а правка человека, сделанная до выгрузки, остаётся изменением.

Записанным считается файл, время изменения которого не раньше начала выгрузки; начало берётся
по часам файловой системы набора. Правку, сделанную во время выгрузки, от записи платформы так
не отличить, и она тоже запоминается — это остаётся задачей #170.

Отказ или отмена выгрузки, правка базы во время неё, неизвестное поколение и отсутствие памяти
о базе память не меняют: следующий `push` грузит записанное выгрузкой. Память полной выгрузки
описывает `INV.USE-CASES.FULL-PULL-RECORDS-THE-PUBLISHED-TREE`, содержимое как основание
решения — `INV.USE-CASES.CHANGES-ARE-DETECTED-ON-DEMAND`.

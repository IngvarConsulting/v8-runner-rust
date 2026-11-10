---
id: INV.USE-CASES.A-SET-HIDDEN-BY-A-GIT-IGNORE-RULE-IS-REFUSED
check:
  - src/change_detection/git_ignore.rs::a_rule_that_meets_no_dumped_file_hides_nothing
  - src/change_detection/git_ignore.rs::an_unanchored_rule_hides_a_remembered_file_at_any_depth
  - src/change_detection/git_ignore.rs::an_unanchored_directory_rule_hides_the_folders_of_a_dump
  - src/change_detection/git_ignore.rs::without_memory_the_rule_is_tried_on_the_files_a_dump_writes
  - src/change_detection/git_ignore.rs::a_rule_anchored_outside_the_root_hides_nothing
  - src/change_detection/git_ignore.rs::a_rule_anchored_above_the_root_hides_the_root
  - src/change_detection/git_ignore.rs::a_negation_lets_the_set_through
  - src/change_detection/git_ignore.rs::a_rule_of_a_gitignore_inside_the_set_counts_from_its_folder
  - src/change_detection/git_ignore.rs::the_version_file_rule_hides_nothing_the_runner_reads
  - src/use_cases/source_changes.rs::an_ignore_rule_hiding_a_remembered_file_refuses_the_command
  - src/use_cases/source_changes.rs::a_gitignore_inside_the_set_refuses_and_the_version_file_rule_does_not
  - tests/cli_status.rs::a_git_ignore_rule_hiding_a_set_file_refuses_the_command
---

# Набор, файл которого прячет правило игнорирования git, получает отказ

Изменения набора в рабочей копии git берутся у git (`INV.USE-CASES.CHANGES-ARE-DETECTED-ON-DEMAND`),
а о файле, который он игнорирует, git молчит: правка такого файла не дошла бы до базы.
Поэтому команда, которая ищет изменения или пишет и читает набор целиком, — `status`, `push`
(и `push --full`), `pull` (и `pull --force`), `make` — отказывает, если правило игнорирования
задевает файл, который пишет платформа. Отказ рода `validation` с кодом
`source_set_ignored_by_git`, код выхода 2; он называет правило, файл и строку, где оно
записано, набор и первые задетые пути — до пяти — и советует привязать правило к нужному
месту, например `/build/` вместо `build/`, или убрать его. Проект без git и исходники формата
EDT не затронуты.

Правила читаются из отслеживаемых `.gitignore` (их называет `git ls-files`), из
`.git/info/exclude` и из файла `core.excludesFile` и сопоставляются, как их сопоставляет git:
решает последнее совпавшее правило, отрицание `!` пропускает путь, игнорируемый каталог
прячет всё под собой, правила `.gitignore` внутри поддерева относятся к своему каталогу.
Правило без привязки (`*.log`, `build/`) задевает любую глубину; правило с привязкой к пути
внутри корня набора или выше него задевает его; правило с привязкой вне корня — нет.

Задетым считается файл, который пишет платформа: у набора с памятью о базе — путь из памяти,
то есть то, что выгрузила платформа, или сам корень набора; без памяти — образцы путей
выгрузки: файлы расширений `.xml`, `.bsl`, `.bin`, `.txt`, `.html`, `.htm` и картинок в
каталогах видов метаданных и вложенных элементов из словаря частичной загрузки. Правило,
которое задевает только то, что раннер и так не читает, — `ConfigDumpInfo.xml`, `workPath`
внутри корня, служебные и порождённые каталоги, — отказа не даёт.

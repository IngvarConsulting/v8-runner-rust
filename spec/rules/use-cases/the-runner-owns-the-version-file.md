---
id: INV.USE-CASES.THE-RUNNER-OWNS-THE-VERSION-FILE
check:
  - tests/cli_pull_memory.rs::a_foreign_version_file_between_commands_does_not_reach_the_dump
  - tests/cli_pull_memory.rs::a_failed_pull_does_not_change_the_runner_copy
  - tests/cli_pull_memory.rs::a_push_refreshes_the_runner_copy
  - tests/cli_pull_memory.rs::a_push_that_writes_no_version_file_keeps_the_runner_copy
  - src/use_cases/version_file.rs::a_replaced_file_gives_way_to_the_runner_copy
  - src/use_cases/version_file.rs::a_missing_file_is_not_restored
  - src/use_cases/version_file.rs::a_copy_of_another_pair_is_not_restored
  - src/use_cases/version_file.rs::a_load_that_did_not_rewrite_the_file_keeps_the_copy
---

# Файл версий принадлежит раннеру

Раннер держит копию `ConfigDumpInfo.xml` набора формата Конфигуратора в
`workPath/infobases/<имя базы>/dump-info/<набор>/` вместе с тождеством памяти набора, для
которого она записана. Файл в каталоге исходников расходный: он остаётся там после
команды, лежит в игноре и служит ручной выгрузке из Конфигуратора.

Перед выгрузкой по изменившемуся — и у `ibcmd`, выгружающего выборку как `--sync`, —
раннер сверяет файл в каталоге с копией. Подменённый файл (содержимое не совпадает с
копией) раннер заменяет копией до запуска платформы. Отсутствующий файл копией не
заменяется: о каталоге, из которого опись пропала, копия ничего не доказывает, и это
случай `INV.USE-CASES.A-MISSING-VERSION-FILE-IS-KNOWN-BEFORE-THE-PLATFORM-STARTS`. Копия
с другим тождеством своей не считается. Правки исходников полной выгрузки не вызывают.

После удачной выгрузки, полной или по изменившемуся, и после удачной загрузки `push`,
переписавшей файл в каталоге, копией становится то, что записала платформа. Перед
загрузкой Конфигуратором или агентом `push` сверяет файл так же, как выгрузка: иначе
частичная загрузка обновила бы чужую опись. Загрузка, которая файла версий не
пишет (`ibcmd config import`), копию не меняет. Сбой команды копию не меняет. Неудачная
запись копии после удачной команды — предупреждение в ответе, а не отказ: прежняя копия
ведёт к выгрузке лишнего, а не к пропуску изменений. Выборочная выгрузка Конфигуратора
копию не меняет.

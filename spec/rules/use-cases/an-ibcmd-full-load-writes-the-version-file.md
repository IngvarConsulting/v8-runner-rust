---
id: INV.USE-CASES.AN-IBCMD-FULL-LOAD-WRITES-THE-VERSION-FILE
check:
  - tests/cli_push_generation.rs::a_full_ibcmd_push_writes_the_version_file_and_the_next_pull_goes_by_it
  - tests/cli_push_generation.rs::a_version_file_the_base_moved_under_is_removed_after_an_ibcmd_push
---

# После полной загрузки `ibcmd` раннер пишет файл версий

`ibcmd config import` файла версий в каталоге набора не пишет, и там осталась бы опись базы до
загрузки. Поэтому после удачной полной загрузки основной конфигурации через `ibcmd` раннер
выгружает файл версий командой `ibcmd config export info` во временный каталог под `workPath`
и читает поколение до этой выгрузки и после. Поколение то же — файл ложится в каталог набора
и становится копией раннера. Поколение другое, ответа нет, команду отменили или выгрузка не
удалась — прежний файл из каталога и копия раннера убираются, и следующий `pull` идёт полным;
ответ это говорит и называет отмену, а загрузка остаётся удачной. Частичная загрузка `ibcmd` и загрузка набора расширения файла версий не
трогают.

## Замер

`ibcmd config export info` пишет тот же `ConfigDumpInfo.xml`, что полная выгрузка
Конфигуратора и `ibcmd`
([замер](../../../references/1c/confirmed-runtime-measurements.md), раздел «Версия формата
файла версий»); `--out` — каталог, а не файл. У расширения команда не замерена.

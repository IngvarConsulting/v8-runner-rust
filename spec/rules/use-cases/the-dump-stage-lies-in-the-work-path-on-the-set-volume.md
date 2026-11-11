---
id: INV.USE-CASES.THE-DUMP-STAGE-LIES-IN-THE-WORK-PATH-ON-THE-SET-VOLUME
check:
  - src/use_cases/dump_config.rs::the_full_dump_stage_lies_in_the_work_path_and_lands_on_the_set
  - src/use_cases/dump_config.rs::a_set_on_another_volume_than_the_work_path_is_refused_before_anything_is_written
  - src/use_cases/dump_config.rs::an_ibcmd_full_forecast_dumps_through_the_stage_without_sync
---

# Промежуточный каталог выгрузки лежит в `workPath` на томе набора

Промежуточный каталог выгрузки — полной выгрузки Конфигуратора и агента, полной выгрузки
`ibcmd` через стадию и импорта EDT в каталог набора — лежит в `workPath/stage`, а не рядом с
набором: во время выгрузки в дереве исходников нет `.dump-stage-*`. Публикация остаётся
переименованием, поэтому стадия и каталог набора на одном томе.

Если каталог набора — у EDT и его снимок Конфигуратора — и `workPath` на разных томах,
`pull` отказывает до выгрузки кодом `work_path_on_another_volume` рода `validation`, выход 2:
в каталог набора и в стадию ничего не записано. Выход — перенести набор или `workPath` на
один том. Внешняя стадия полной выгрузки лежит у вызывающего, и её том он сверяет сам.
Замок выгрузки остаётся рядом с целью
(`INV.USE-CASES.A-DUMP-LOCK-LEAVES-NO-FILES-IN-THE-SOURCE-TREE`).

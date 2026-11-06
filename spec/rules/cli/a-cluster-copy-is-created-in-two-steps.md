---
id: INV.CLI.A-CLUSTER-COPY-IS-CREATED-IN-TWO-STEPS
check:
  - tests/cli_infobase_copy.rs::a_cluster_copy_is_created_by_the_designer_and_loaded_from_the_image
  - tests/cli_infobase_copy.rs::a_failed_load_into_the_cluster_names_the_image_and_hides_the_passwords
---

# Копию базы в кластере Конфигуратор создаёт в два шага

`infobase create --from` создаёт базу в кластере двумя вызовами Конфигуратора:
`CREATEINFOBASE`, затем `/RestoreIB` с образом источника. Шаблон
`CREATEINFOBASE … /UseTemplate <файл>.dt` раннер не использует.

При шаблоне причину сбоя загрузки образа сообщает только `/Out` вызова `CREATEINFOBASE`, а этот
файл повторяет `DBPwd` и `SPwd` и при отказе, поэтому раннер `/Out` этому вызову не передаёт.
При двух шагах сбой загрузки сообщает `/RestoreIB`, чей `/Out` паролей не содержит.

[Замер](../../../references/1c/confirmed-runtime-measurements.md#createinfobase-с-dt-шаблоном-в-кластере):
данные и расширение оба способа переносят одинаково, и оба оставляют зарегистрированную ИБ и БД
после отказа на усечённом образе. На занятой непустой БД шаблон отказывает и сохраняет данные, а
`/RestoreIB` их заменяет; об этом предупреждают превью и ответ
(`INV.CLI.INFOBASE-CREATE-FROM-COPIES-A-BASE`).

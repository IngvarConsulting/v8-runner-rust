---
id: INV.CLI.A-CLUSTER-COPY-IS-CREATED-IN-TWO-STEPS
check:
  - tests/cli_infobase_copy.rs::a_cluster_copy_is_created_by_the_designer_and_loaded_from_the_image
  - tests/cli_infobase_copy.rs::a_failed_load_into_the_cluster_names_the_image_and_hides_the_passwords
  - src/platform/designer.rs::create_cluster_infobase_passes_the_string_and_requests_the_configured_log
---

# Копию базы в кластере Конфигуратор создаёт в два шага

`infobase create --from` создаёт базу в кластере двумя вызовами Конфигуратора:
`CREATEINFOBASE`, затем `/RestoreIB` с образом источника. Шаблон
`CREATEINFOBASE … /UseTemplate <файл>.dt` раннер не использует.

При шаблоне причину сбоя загрузки образа сообщает только `/Out` вызова `CREATEINFOBASE`.
При двух шагах о сбое создания сообщает `CREATEINFOBASE`, а о сбое загрузки — `/RestoreIB`.
Журнал первого шага может повторять `DBPwd` и `SPwd`, поэтому раннер читает его из закрытого
временного каталога и маскирует пароли в ответе. В диагностике второго шага раннер также
маскирует пароль пользователя базы.

[Замер](../../../references/1c/confirmed-runtime-measurements.md#createinfobase-с-dt-шаблоном-в-кластере):
в сверенных таблицах и расширении результат у двух способов совпал, повторные `.dt` различаются.
Оба оставляют зарегистрированную ИБ и БД после отказа на усечённом образе. На занятой непустой БД
шаблон с реквизитами администратора кластера отказал при аутентификации и данные сохранил, а
`/RestoreIB` их заменил; об этом предупреждают превью и ответ
(`INV.CLI.INFOBASE-CREATE-FROM-COPIES-A-BASE`).

---
id: INV.CLI.A-PACKAGE-DIRECTION-OF-CONVERT-HAS-AN-EXECUTOR-CHAIN
check:
  - tests/cli_convert.rs::convert_a_set_to_a_package_builds_it_with_ibcmd_in_a_throwaway_base
  - tests/cli_convert.rs::convert_a_package_file_to_xml_exports_it_in_a_throwaway_base
  - tests/cli_convert.rs::convert_a_package_preview_dispatches_nothing
  - tests/cli_convert.rs::convert_a_package_direction_without_ibcmd_answers_an_environment_failure
  - tests/cli_convert.rs::convert_without_source_set_processes_all_source_sets_into_work_path_out
  - src/config/validate.rs::convert_has_no_executor_choice_until_ibcmd_rs_is_measured
---

# Направление `convert` с пакетом выполняет цепочка исполнителей

Направления с пакетом исполняет строка `convert` матрицы, и квитанция `data.provider`
называет выбранного исполнителя — и в ответе, и в превью, и в отказе после выбора. Пока
`ibcmd-rs` не замерен (`INV.USE-CASES.IBCMD-RS-FOLLOWS-IBCMD-IN-THE-CONVERT-CHAIN`), в
строке один `ibcmd`, и ключу `providers.convert` выбирать не из чего: валидация его
отклоняет, не называя вида базы. `ibcmd` работает во временной базе раннера
(`INV.USE-CASES.IBCMD-BUILDS-A-PACKAGE-IN-A-THROWAWAY-BASE`,
`INV.USE-CASES.IBCMD-EXPORTS-A-PACKAGE-IN-A-THROWAWAY-BASE`).

Без `ibcmd` направление с пакетом отказывает родом `environment`
(`INV.WIRE.A-MISSING-TOOL-IS-AN-ENVIRONMENT-FAILURE`), а квитанция называет его
пропущенным с причиной. Перевод между EDT и XML выполняет `1cedtcli`: выбора исполнителя у
него нет, и квитанции в ответе тоже.

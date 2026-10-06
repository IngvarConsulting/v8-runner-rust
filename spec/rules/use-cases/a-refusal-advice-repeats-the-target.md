---
id: INV.USE-CASES.A-REFUSAL-ADVICE-REPEATS-THE-TARGET
check:
  - src/use_cases/destruction_guard.rs::the_pull_advice_repeats_the_target_with_the_global_keys_of_the_run
  - src/use_cases/context.rs::a_command_line_names_the_global_keys_of_the_target
  - src/use_cases/context.rs::an_advice_without_a_config_path_names_the_project_directory
  - src/use_cases/dump_config.rs::an_edt_extension_refusal_does_not_offer_a_bare_pull_force
  - src/use_cases/dump_config.rs::an_mcp_extension_refusal_names_the_exact_pull_for_the_same_source_set
  - tests/cli_dump.rs::an_mcp_refusal_advises_the_command_line_of_the_same_base_and_workdir
  - tests/mcp_http.rs::mcp_http_refusal_advises_the_command_line_of_the_server_target
  - tests/cli_pull_memory.rs::foreign_memory_is_named_in_the_response_without_dispatching_or_exposing_credentials
  - tests/cli_pull_memory.rs::foreign_memory_advice_runs_as_written_against_the_same_base
  - tests/cli_convert.rs::a_convert_refusal_does_not_offer_a_truncated_command
  - tests/architecture_guardrails.rs::no_production_text_advises_a_bare_pull_force_or_push_full
---

# Совет отказа повторяет исходную цель

Команда, которую отказ советует выполнить, бьёт в ту же цель, что исходный вызов: она
несёт набор исходников и глобальные ключи, которыми достигнута цель вызова. Совет без
набора (`pull --force`, `push --full`) отказ не даёт: буквальный повтор выгрузил бы набор
по умолчанию или загрузил бы все наборы.

Команда `pull <SET> --force` в совете называет набор так, как его принимает позиционный
аргумент, конфиг — абсолютным путём `--config`, базу — `--infobase` с именем из проекта,
если выбрана не база по умолчанию, рабочий каталог — `--workdir`, если он переопределён.
Так совет, выполненный из другого каталога или клиентом сервера MCP, запущенного с
`--infobase`, попадает в тот же проект и ту же базу. У MCP по HTTP совет оговаривает, что
команду выполняют на машине, где работает сервер. Значения закавычены для оболочки
платформы: POSIX на Unix, двойными кавычками для PowerShell и `cmd` на Windows.

Где путь конфига не разрешился, совет не выдаёт команду без `--config` за готовую, а
велит выполнить её из каталога проекта.

Где точной команды собрать не из чего (`convert`), совет — тот же вызов с добавленным
ключом, со всеми его аргументами, а не команда, урезанная до имени.

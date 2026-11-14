---
id: INV.USE-CASES.A-PLAIN-EXTENSION-SYNONYM-FITS-THE-CONFIGURATION-LANGUAGES
check:
  - src/use_cases/extension_agent.rs::a_plain_synonym_is_wrapped_with_fallbacks_and_an_nstr_one_is_kept
  - tests/cli_agent_scenarios.rs::extensions_create_without_a_synonym_fits_a_configuration_without_en
  - tests/cli_agent_scenarios.rs::extensions_create_without_a_synonym_fits_a_configuration_without_ru
  - tests/cli_agent_scenarios.rs::extensions_create_without_a_shipped_language_names_the_way_out
  - tests/cli_agent_scenarios.rs::extensions_create_does_not_rewrite_a_synonym_with_named_languages
  - tests/cli_agent_scenarios.rs::a_cancellation_during_a_language_refusal_stops_the_synonym_forms
  - tests/cli_agent_standalone.rs::gate_create_reads_languages_before_its_only_mutation
  - tests/cli_agent_standalone.rs::gate_refuses_an_absent_named_language_without_changing_inventory
  - tests/cli_agent_standalone.rs::gate_forwards_a_supported_named_synonym_once_even_if_creation_fails
  - tests/cli_agent_standalone.rs::a_gate_create_argument_cannot_start_another_shell_command
  - tests/cli_agent_standalone.rs::gate_create_refuses_unreadable_language_metadata
  - tests/cli_agent_standalone.rs::gate_create_does_not_export_or_mutate_after_losing_sftp
  - tests/cli_agent_standalone.rs::cancellation_during_gate_language_export_prevents_creation
---

# Простой синоним расширения подстраивается под языки конфигурации

`extensions create` через агента проходит и там, где в конфигурации есть только один из языков
`ru` и `en`. Синоним без языка — и имя вместо него — раннер передаёт в форме `NStr()` с доступными
кодами из `ru` и `en`. Если в конфигурации нет подходящего кода, отказ предлагает задать
`--synonym` с языком конфигурации.

У агента Конфигуратора порядок попыток — `ru` и `en` вместе, затем `ru`, затем `en`. Переход к
следующей форме разрешён после структурного `LanguageNotFound`, при котором расширение не создано.
Строка с названными языками передаётся один раз в исходном виде. Отмена останавливает перебор.

Шлюз `ibsrv` может оставить частичную запись даже при отказе по языку. Раннер сначала читает
языки рабочей конфигурации в той же подключённой SSH-сессии по XML выгрузки, используя объявленный
канал обмена. Явная форма `NStr()` передаётся в исходном виде, когда каждый указанный в ней код
есть в конфигурации. Отсутствующий код, недоступность языков или отмена до отправки создания
оставляют состав расширений прежним. Создание через шлюз отправляется не более одного раза.
Решения опираются на артефакты и структурные ответы платформы.

Защита от повторного появления дефекта: проверка языков шлюза принадлежит сценарию создания
расширения. Регрессия проверяет команды той же сессии, отсутствие записи при недоступном языке
и число попыток создания при ответе шлюза без `error-type`.

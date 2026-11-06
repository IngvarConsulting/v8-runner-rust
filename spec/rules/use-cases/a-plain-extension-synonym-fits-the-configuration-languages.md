---
id: INV.USE-CASES.A-PLAIN-EXTENSION-SYNONYM-FITS-THE-CONFIGURATION-LANGUAGES
check:
  - src/use_cases/extension_agent.rs::a_plain_synonym_is_wrapped_with_fallbacks_and_an_nstr_one_is_kept
  - tests/cli_agent_scenarios.rs::extensions_create_without_a_synonym_fits_a_configuration_without_en
  - tests/cli_agent_scenarios.rs::extensions_create_without_a_synonym_fits_a_configuration_without_ru
  - tests/cli_agent_scenarios.rs::extensions_create_without_a_shipped_language_names_the_way_out
  - tests/cli_agent_scenarios.rs::extensions_create_does_not_rewrite_a_synonym_with_named_languages
  - tests/cli_agent_scenarios.rs::a_cancellation_during_a_language_refusal_stops_the_synonym_forms
---

# Простой синоним расширения подстраивается под языки конфигурации

`extensions create` через агента проходит и там, где в конфигурации есть только один из языков
`ru` и `en`. Агент требует синоним в форме `NStr()` и отказывает `LanguageNotFound` на коде вне конфигурации,
ничего не создав, а узнать языки конфигурации у него нечем. Поэтому синоним без языка — и имя
вместо него — уходит формой `ru` и `en`, а при таком отказе формой `ru`, затем `en`. Решение
принимается по роду отказа, а не по тексту. Если не подошла ни одна, отказ называет выход: задать
`--synonym` с языком конфигурации. Строку, где языки названы, раннер не переписывает и отправляет
один раз. Отмена, пришедшая во время перебора, его останавливает.

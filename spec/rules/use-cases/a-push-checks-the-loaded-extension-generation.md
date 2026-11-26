---
id: INV.USE-CASES.A-PUSH-CHECKS-THE-LOADED-EXTENSION-GENERATION
check:
  - tests/cli_push_generation.rs::an_ibcmd_extension_push_requires_an_answer_that_matches_its_baseline
  - tests/cli_push_generation.rs::a_legacy_designer_extension_record_cannot_authorize_an_ibcmd_push
  - tests/cli_build_agent.rs::an_ibcmd_baseline_does_not_allow_an_unforced_agent_extension_load
  - tests/cli_build_agent.rs::a_forced_extension_push_connects_and_loads_through_the_agent
---

# Отправка расширения сверяет загруженную конфигурацию

Загрузка набора расширения проекта через `push` без `--force` разрешена только через `ibcmd`
при совпадении сохранённого и текущего токенов загруженной конфигурации от `ibcmd`.
Это относится и к загрузке без применения. Другой исполнитель, отсутствие записи `ibcmd`,
недоступное поколение и расхождение приводят к `non_fast_forward` до первой загрузки.
`status --deep` сообщает `moved_ahead` при доказанном расхождении и `no_answer` с причиной
при невозможности безопасного сравнения. Превью не спрашивает поколение у платформы.

`--force` обходит сверку поколения у каждого исполнителя, включая `ibcmd`. Проверки
доступа и замки действуют. Агент и пакетный Конфигуратор читают применённое поколение
расширения. Эти токены не допускают обычную загрузку расширения.

Граница правила — загрузка наборов проекта. Применение уже загруженной конфигурации
и подготовка инструментального расширения Client MCP описаны своими правилами.

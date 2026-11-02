---
id: INV.PLATFORM.A-PLATFORM-WITHOUT-THE-GENERATION-COMMAND-IS-NOT-ASKED
check:
  - src/platform/generation.rs::a_platform_older_than_8_3_20_knows_no_generation_command
  - src/platform/designer.rs::a_platform_without_the_generation_command_is_not_asked
  - src/platform/ibcmd.rs::a_platform_without_the_generation_command_is_not_asked
---

# Платформу без команды поколения о поколении не спрашивают

Команда поколения — `/GetConfigGenerationID` пакетного Конфигуратора и `config
generation-id` режима агента — реализована в 8.3.20 (список изменений платформы 8.3.20).
Платформу старше о поколении не спрашивают: пакетной операции в запуске Конфигуратора нет,
он уходит в интерактивный режим и сам не кончается
([замер](../../../references/1c/confirmed-runtime-measurements.md), 03.11.2026, 8.3.12).
Такой запуск — не отказ и не отсутствие ответа, которых ждёт разбор, а зависание, —
поэтому вопрос не задают вовсе.

Версия платформы читается из пути утилиты, как её читает и поиск утилит; версии в пути нет —
вопрос задают: от запуска без конца чтение защищено собственным пределом
(`INV.USE-CASES.A-GENERATION-READ-DECLARES-ITS-OWN-CAP`), а у `ibcmd` незнакомая подкоманда
и так отвечает отказом.

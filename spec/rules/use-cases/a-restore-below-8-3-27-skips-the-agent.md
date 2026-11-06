---
id: INV.USE-CASES.A-RESTORE-BELOW-8-3-27-SKIPS-THE-AGENT
check:
  - tests/cli_minimum_platform.rs::a_restore_below_8_3_27_takes_the_designer_and_from_8_3_27_the_agent
  - tests/cli_minimum_platform.rs::a_dump_below_8_3_27_stays_with_the_agent
  - tests/cli_minimum_platform.rs::an_assigned_agent_below_8_3_27_refuses_the_restore_with_the_reason
---

# `infobase restore` ниже 8.3.27 не выбирает агента

Агент платформ ниже 8.3.27 в `infobase-tools restore-ib` просит разрешение на перезапуск. Раннер
на вопросы агента не отвечает, поэтому команда падала бы с кодом 4 и состоянием цели `uncertain`
([замер](../../../references/1c/confirmed-runtime-measurements.md#раннер-на-83248326-граница-минимальной-версии)
на 8.3.24 и 8.3.26). Конфигуратор `/RestoreIB` там работает.

Поэтому у такой платформы агент при выборе исполнителя пропускается с причиной в `skipped`, и
цепочка идёт к Конфигуратору, в превью так же. Ключ `providers.infobase.restore: agent` назначает
одного исполнителя без отката: подмены нет, команда отказывает до запуска платформы и называет
причину и выход.

Остальные команды агента граница не касается. Там, где версия не видна, агент остаётся в цепочке:
утилита без каталога версии в пути и чужой агент (`attached`).

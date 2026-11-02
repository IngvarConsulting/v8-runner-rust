---
id: INV.USE-CASES.A-GENERATION-READ-DECLARES-ITS-OWN-CAP
check:
  - src/platform/designer.rs::a_generation_read_past_its_cap_is_no_answer
  - src/platform/ibcmd.rs::a_generation_read_past_its_cap_is_no_answer
  - src/platform/designer.rs::another_spawn_failure_stays_a_failure
  - src/platform/ibcmd.rs::another_spawn_failure_stays_a_failure
---

# Чтение поколения объявляет собственный предел

У команды срока нет (`DEC.2026-09-20.A-COMMAND-HAS-NO-DEADLINE`), но чтение поколения —
вопрос, а не работа: у платформы, которой команда поколения не знакома, у Конфигуратора
нет пакетной операции, и его запуск конца не имеет. Чтение объявляет собственный предел
(`generation_reader::GENERATION_PROBE_CAP`) — шагу его объявлять разрешено, команда
срока по-прежнему не имеет. Запуск, снятый по пределу, — отсутствие ответа, а не отказ:
разбор ответа ждёт и его. Отмена и прочие отказы запуска остаются отказами.

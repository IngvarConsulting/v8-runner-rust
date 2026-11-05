---
id: INV.USE-CASES.A-GENERATION-READ-DECLARES-ITS-OWN-CAP
check:
  - src/use_cases/generation_reader.rs::the_generation_read_declares_its_own_cap
  - src/use_cases/generation_reader.rs::the_ibcmd_generation_read_declares_its_own_cap
  - src/use_cases/generation_reader.rs::a_cancelled_generation_read_stays_a_failure
  - src/platform/designer.rs::a_generation_read_past_its_cap_is_no_answer
  - src/platform/ibcmd.rs::a_generation_read_past_its_cap_is_no_answer
  - src/platform/designer.rs::another_spawn_failure_stays_a_failure
  - src/platform/ibcmd.rs::another_spawn_failure_stays_a_failure
  - src/use_cases/build_project.rs::an_unanswered_generation_before_a_load_is_named_not_refused
  - src/use_cases/build_project.rs::a_load_without_a_generation_record_is_not_named_unanswered
---

# Чтение поколения объявляет собственный предел

У команды срока нет (`DEC.2026-09-20.A-COMMAND-HAS-NO-DEADLINE`), но чтение поколения —
вопрос, а не работа: у платформы, которой команда поколения не знакома, у Конфигуратора
нет пакетной операции, и его запуск конца не имеет. Чтение объявляет собственный предел
(`generation_reader::GENERATION_PROBE_CAP`) — шагу его объявлять разрешено, команда
срока по-прежнему не имеет. Запуск, снятый по пределу, — отсутствие ответа, а не отказ:
разбор ответа ждёт и его. Отмена и прочие отказы запуска остаются отказами.

Отсутствие ответа не скрывается. Если у набора есть запись поколения, а база перед
загрузкой не ответила (платформа без команды поколения или чтение снято пределом), `push`
загружает набор, но ответ шага называет несверенное поколение: отправка не проверила, что
база не ушла вперёд с прошлого обмена (`GenerationGate::unanswered_note`).

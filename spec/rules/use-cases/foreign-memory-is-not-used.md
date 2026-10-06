---
id: INV.USE-CASES.FOREIGN-MEMORY-IS-NOT-USED
check:
  - tests/cli_pull_memory.rs::foreign_memory_is_named_in_the_response_without_dispatching_or_exposing_credentials
  - tests/cli_push_generation.rs::a_full_push_with_memory_of_another_base_is_refused_as_no_memory
  - tests/cli_push_generation.rs::a_generation_record_does_not_make_memory_of_another_pair_own
  - src/change_detection/source_sets.rs::base_snapshots_remain_separate_and_reject_a_retargeted_base
  - src/use_cases/version_file.rs::a_copy_of_another_pair_is_not_restored
  - src/use_cases/agent_session.rs::a_generation_recorded_for_another_pair_is_not_used
---

# Чужая память не используется

Память под именем базы, записанная для другой базы, объявляется чужой в ответе и не даёт
основания пропустить работу.

Для `push` хеш-память другой пары — отсутствие памяти о базе, даже если запись поколения
своя: он отказывает `no_memory` до анализа изменений и платформы, с выходами `pull <SET>` и
`push --force` (`INV.USE-CASES.A-PUSH-WITHOUT-MEMORY-OF-THE-BASE-IS-REFUSED`), и у полной
загрузки `--full` тоже. Отказ говорит, что память под этим именем записана для другой базы
или каталога. Решение владельца от 06.10.2026: выход каталога — `push --force`, а не
`--full`, потому что `--full` проверки памяти не обходит.

Остальная память о базе хранит рядом ту же привязку. Копия файла версий другой пары не
подкладывается в каталог (`INV.USE-CASES.A-REPLACED-VERSION-FILE-GIVES-WAY-TO-THE-RUNNER-COPY`).
Запись о поколении другой пары не даёт агенту пропустить выгрузку, и ответ выгрузки
называет её чужой.

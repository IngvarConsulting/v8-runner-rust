---
id: INV.USE-CASES.FOREIGN-MEMORY-IS-NOT-USED
check:
  - tests/cli_pull_memory.rs::foreign_memory_is_named_in_the_response_without_dispatching_or_exposing_credentials
  - src/change_detection/source_sets.rs::base_snapshots_remain_separate_and_reject_a_retargeted_base
  - src/use_cases/version_file.rs::a_copy_of_another_pair_is_not_restored
  - src/use_cases/agent_session.rs::a_generation_recorded_for_another_pair_is_not_used
---

# Чужая память не используется

Память под именем базы, записанная для другой базы, объявляется чужой в ответе и не даёт
основания пропустить работу.

Обычный `push` при чужой хеш-памяти отказывает, называя записанную и выбранную привязки
и выходы с тем же набором: `pull <SET> --force`, если права база, и `push <SET> --full`,
если прав каталог; каждый заменяет память после успеха.

Остальная память о базе хранит рядом ту же привязку. Копия файла версий другой пары не
подкладывается в каталог (`INV.USE-CASES.A-REPLACED-VERSION-FILE-GIVES-WAY-TO-THE-RUNNER-COPY`).
Запись о поколении другой пары не даёт агенту пропустить выгрузку, и ответ выгрузки
называет её чужой.

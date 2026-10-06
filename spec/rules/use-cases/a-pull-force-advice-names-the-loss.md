---
id: INV.USE-CASES.A-PULL-FORCE-ADVICE-NAMES-THE-LOSS
check:
  - src/use_cases/destruction_guard.rs::the_pull_advice_repeats_the_target_with_the_global_keys_of_the_run
  - tests/cli_pull_memory.rs::foreign_memory_advice_runs_as_written_against_the_same_base
  - tests/cli_synonyms.rs::mode_full_is_refused_and_names_pull_force
---

# Совет `pull <SET> --force` называет потерю

Отказ, который советует `pull <SET> --force`, тут же говорит, что эта команда заменяет
каталог набора и теряет незакоммиченное в нём: отказ сторожа замены, отказ прежнего
`--mode` и отказ `push` при чужой памяти. Совет не уводит молча в уничтожение работы,
которую исходный вызов не трогал.

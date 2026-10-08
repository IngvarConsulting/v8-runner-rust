---
id: INV.CLI.CLONE-FROM-WARNS-ON-A-BASE-OF-ANOTHER-COPY
check:
  - tests/cli_bootstrap.rs::clone_from_a_base_of_another_copy_runs_with_a_warning
  - tests/contract_previews.rs::no_preview_changes_what_a_real_build_left_in_the_work_path
---

# `clone --from` на базе другой копии предупреждает

`clone --from` на файловой базе другой рабочей копии не отказывает: прогон заводит проект и
выгружает базу, а ответ и превью несут предупреждение
`INV.USE-CASES.A-WRITE-ON-A-BASE-OF-ANOTHER-COPY-RUNS-WITH-A-WARNING`. Метка остаётся за
прежней копией.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies),
[`cli.html#map`](../../../docs/site/cli.html#map).

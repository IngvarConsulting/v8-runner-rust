---
id: INV.CLI.A-PREVIEW-NAMES-THE-WARNING-ON-A-BASE-OF-ANOTHER-COPY
check:
  - tests/cli_infobase_owner.rs::a_preview_names_the_warning_on_a_base_of_another_copy_and_writes_nothing
  - src/use_cases/infobase_owner.rs::a_preview_on_a_base_of_another_copy_warns_like_the_run
  - tests/contract_previews.rs::no_preview_changes_what_a_real_build_left_in_the_work_path
  - tests/cli_infobase_owner.rs::a_preview_on_a_base_of_another_copy_gives_the_warning_code_and_writes_nothing
---

# Превью называет предупреждение о базе другой копии

Превью команды записи на базе другой рабочей копии несёт то же предупреждение, что и прогон
(`INV.USE-CASES.A-WRITE-ON-A-BASE-OF-ANOTHER-COPY-RUNS-WITH-A-WARNING`), с тем же кодом
`infobase_of_another_copy` в `warning_details`, и ни базу, ни метку не трогает. Так вызывающий
узнаёт до записи, что база чужая, и может спросить человека.
Команды записи и чтения определены в
`INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).

---
id: INV.PLATFORM.PROCESS-OUTPUT-IS-HELD-WITHIN-A-BOUND
check:
  - src/platform/process/capture.rs::a_flood_of_output_is_kept_within_the_bound
  - src/platform/process/capture.rs::one_huge_chunk_is_kept_within_the_bound
  - src/platform/process/capture.rs::output_within_the_bound_is_kept_as_written
  - src/platform/process/capture.rs::a_cut_inside_a_character_keeps_both_parts_readable
  - tests/architecture_guardrails.rs::process_output_has_one_bounded_reader_and_ibcmd_one_guarded_launch
---

# Вывод процесса раннер держит в пределе

Разовый процесс, чей вывод раннер собирает (`ProcessRunner::run_with_policy`), читается
только через `src/platform/process/capture.rs::read_stream`. Из каждого потока, stdout и
stderr, раннер держит в памяти не больше первого мебибайта и последних семи. Середина
опускается, а на её месте стоит отдельная строка
`[v8-runner: <N> bytes of output omitted]`. Её не примет за свою строку ни один разборщик
вывода, поэтому разбор, закрытый от неизвестного, откажет, а не прочтёт половину. Ответ, который
утилита пишет последним, например токен поколения, остаётся в конце.

Срез идёт по концу строки, если тот лежит в 64 КиБ от среза. Иначе он сдвигается к
границе символа UTF-8, если вывод в UTF-8. Начало и конец переводятся из кодовой страницы
порознь. Журнал потока (`stdout_log_path`, `stderr_log_path`) получает те же байты, что
держит раннер.

Причина — #742: `ibcmd`, спрашивавший без конца, за 43 минуты набрал раннеру 2,1 ГБ
памяти.

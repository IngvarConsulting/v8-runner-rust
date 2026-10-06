---
id: INV.CONFIG.A-DECLARED-SOURCE-SET-KEEPS-THE-PROJECT-FILE-TEXT
check:
  - src/use_cases/config_init.rs::a_declared_set_is_appended_and_the_rest_of_the_text_is_kept
  - src/use_cases/config_init.rs::a_declared_set_follows_the_layout_of_the_sequence
  - src/use_cases/config_init.rs::a_sequence_without_a_block_layout_is_refused_not_rewritten
  - tests/cli_pull_all.rs::an_extension_without_a_set_is_declared_and_pulled
---

# Объявленный набор дописывается, остальной текст проекта не меняется

Набор, который раннер объявляет в существующем `v8project.yaml`, дописывается в конец
блочной последовательности `source-set:` с её отступом и переводом строки. Комментарии,
порядок ключей и вид остального текста остаются как были. Дописанный текст читается заново
и должен дать прежний документ с добавленными наборами; когда это не так — например,
последовательность записана потоком, — раннер файл не переписывает, а отказывает и называет
записи, которые надо внести руками.

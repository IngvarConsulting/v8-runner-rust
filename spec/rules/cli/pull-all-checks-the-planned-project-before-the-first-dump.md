---
id: INV.CLI.PULL-ALL-CHECKS-THE-PLANNED-PROJECT-BEFORE-THE-FIRST-DUMP
check:
  - tests/cli_pull_all.rs::a_declaration_that_breaks_the_project_is_refused_before_any_dump
  - tests/cli_pull_all.rs::an_extension_named_like_another_set_is_refused_before_any_dump
  - tests/cli_pull_all.rs::a_project_file_that_cannot_take_the_declaration_is_refused_before_any_dump
  - src/config/validate.rs::a_declared_set_that_would_break_the_project_is_refused
  - src/config/validate.rs::a_declared_set_without_its_directory_yet_passes_the_project_checks
  - src/use_cases/dump_config/all.rs::a_name_taken_by_a_set_of_another_purpose_is_refused
---

# План `pull --all` проверяется до первой выгрузки

Прежде чем выгрузить первый набор, `pull --all` проверяет проект таким, каким он станет с
объявляемыми наборами, теми же проверками, что проект при загрузке: имя и путь набора,
совпадение каталога с каталогом другого набора после канонизации, имена, которые держит за
собой EDT, имя расширения-инструмента. Каталога объявляемого набора может ещё не быть, и
этого от него не требуют. Отказ идёт и тогда, когда имя расширения занято набором другого
назначения без учёта регистра, и когда запись в проектный файл не дописать. Ничего не
выгружено, проектный файл не тронут.

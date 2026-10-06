---
id: INV.CLI.A-SYNONYM-ANSWERS-UNDER-THE-NEW-NAME
check:
  - tests/cli_synonyms.rs::every_previous_name_answers_as_its_dictionary_entry
  - tests/cli_synonyms.rs::every_synonym_of_the_table_has_a_case
---

# Синоним отвечает новым именем

Команда, вызванная прежним именем, отвечает конвертом с новым именем в поле `command`, и
квитанция прежнего имени не упоминает.

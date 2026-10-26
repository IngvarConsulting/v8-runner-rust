---
id: INV.USE-CASES.FORCE-NAMES-WHAT-IT-DESTROYED
check:
  - src/use_cases/destruction_guard.rs::consent_names_what_it_destroys
  - tests/cli_dump.rs::force_names_what_it_destroyed
  - tests/cli_dump.rs::an_edt_project_is_replaced_only_by_the_same_confirmation_rules
  - tests/cli_convert.rs::a_convert_refusal_does_not_offer_a_truncated_command
  - tests/cli_external_full_pull.rs::accepted_external_full_pull_remembers_the_original_source_for_the_next_push
---

# `--force` называет уничтоженное

Ответ на `--force` называет уничтоженное: сколько путей и первые из них по имени — и когда
система контроля версий нашла безвозвратное, и когда ответа у неё нет: тогда уничтожен
каждый файл каталога. Зафиксированное уничтоженным не называется.

У `pull` это поле `losses` формы `CTR.WIRE.PULL-DATA` и сообщение, у `convert` — сообщение.
Замену, которую публикует вызывающий (`--publication-stdio`), ответ называет так же. Перечень
составляет вопрос к сторожу перед самой публикацией замены.

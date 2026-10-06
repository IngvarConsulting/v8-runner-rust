---
id: INV.USE-CASES.FORCE-NAMES-WHAT-IT-DESTROYED
check:
  - src/use_cases/destruction_guard.rs::consent_names_what_it_destroys
  - tests/cli_dump.rs::force_names_what_it_destroyed
  - tests/cli_dump.rs::an_edt_project_is_replaced_only_by_the_same_confirmation_rules
  - tests/cli_convert.rs::a_convert_refusal_does_not_offer_a_truncated_command
---

# `--force` называет уничтоженное

Ответ на `--force` перечисляет уничтоженное поимённо — и когда система контроля версий
нашла безвозвратное, и когда ответа у неё нет: тогда уничтожен каждый файл каталога.
Зафиксированное уничтоженным не называется.

Полный перечень у `pull` несёт поле `losses` формы `CTR.WIRE.PULL-DATA`, сообщение называет
первые имена и счёт остальных; у `convert` перечень несёт сообщение. Перечень составляет
вопрос к сторожу перед самой публикацией замены. `pull --force` всегда замена; перезапись
поверх каталога лишнего не удаляет и уничтоженным ничего не называет.

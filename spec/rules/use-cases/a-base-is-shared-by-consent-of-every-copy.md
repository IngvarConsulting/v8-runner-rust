---
id: INV.USE-CASES.A-BASE-IS-SHARED-BY-CONSENT-OF-EVERY-COPY
check:
  - tests/cli_infobase_owner.rs::a_base_shared_by_every_holder_takes_writes_of_each
  - tests/cli_infobase_owner.rs::a_base_without_consent_of_one_holder_is_refused_and_names_it
  - tests/cli_infobase_owner.rs::a_second_section_of_the_base_without_consent_withdraws_it
---

# Общей базу делает согласие каждой копии

Если у файловой базы `shared: true` стоит в местном слое каждой держащей её рабочей копии и
этой, проверка владельца пропускает команду записи, и копия записывается в метку рядом с
остальными. Команды записи и чтения определены в
`INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).

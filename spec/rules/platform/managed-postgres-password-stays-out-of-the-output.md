---
id: INV.PLATFORM.MANAGED-POSTGRES-PASSWORD-STAYS-OUT-OF-THE-OUTPUT
check:
  - src/use_cases/managed_stand/mod.rs::the_password_stays_out_of_the_answer_and_the_arguments
---

# Пароль управляемого PostgreSQL не попадает в вывод

Пароль лежит в файле рядом с записью, а `dbms status` называет путь к файлу, но не значение. В ответы и журналы раннера
значение не попадает, включая содержимое `/Out`. Командная строка `CREATEINFOBASE` и `rac` пароль несёт: stdin у них для
него нет, и процесс виден пользователям этой машины. Сайт это называет.

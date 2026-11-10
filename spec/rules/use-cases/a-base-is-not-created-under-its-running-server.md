---
id: INV.USE-CASES.A-BASE-IS-NOT-CREATED-UNDER-ITS-RUNNING-SERVER
check:
  - src/config/loader.rs::infobase_create_is_refused_under_the_running_server
---

# База автономного сервера не создаётся под работающим сервером

При работающем своём `ibsrv` этой базы `infobase create`, и с `--recreate` тоже, отказывает до
любых изменений и называет `standalone stop` полем `next`: `ibsrv` держит подключения к своей
БД и файлы своей файловой базы (замер [#597](../../../references/1c/confirmed-runtime-measurements.md)).

---
id: INV.USE-CASES.A-BASE-IS-NOT-CREATED-UNDER-ITS-RUNNING-SERVER
check:
  - src/config/loader.rs::infobase_create_is_refused_under_the_running_server
---

# База автономного сервера не создаётся под работающим сервером

При работающем своём `ibsrv` этой базы `infobase create` отказывает и называет `standalone stop`.

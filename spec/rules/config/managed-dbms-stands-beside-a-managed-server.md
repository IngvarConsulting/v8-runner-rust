---
id: INV.CONFIG.MANAGED-DBMS-STANDS-BESIDE-A-MANAGED-SERVER
check:
  - src/config/validate.rs::a_managed_dbms_needs_a_managed_server_beside_it
---

# Управляемая СУБД объявляется только рядом с управляемым сервером

`dbms.managed` принимается рядом с `cluster.managed` или `standalone.managed`. Рядом с кластером по строке
соединения, с автономным сервером, который поднимаете вы, и с файловой базой — отказ валидации: раннер не
может ни создать у чужого сервера доступ к своей СУБД, ни отвечать за такой порядок запуска.

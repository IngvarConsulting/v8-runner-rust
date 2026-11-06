---
id: INV.CONFIG.MANAGED-DBMS-STANDS-BESIDE-A-MANAGED-SERVER
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/536
---

# Управляемая СУБД объявляется только рядом с управляемым сервером

`dbms.managed` принимается рядом с `cluster.managed` или `standalone.managed`. Рядом с кластером по строке
соединения, с автономным сервером, который поднимаете вы, и с файловой базой — отказ валидации: раннер не
может ни создать у чужого сервера доступ к своей СУБД, ни отвечать за такой порядок запуска.

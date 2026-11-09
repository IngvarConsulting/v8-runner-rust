---
id: INV.PLATFORM.STANDALONE-PORTS-AND-KEY-ARE-PINNED
check:
  - src/use_cases/managed_stand/standalone.rs::the_ports_and_the_key_stay_from_start_to_start
---

# Порты и ключ автономного сервера закреплены

Блок портов и ключ хоста SSH-шлюза создаются при первом запуске и остаются от запуска к запуску: адрес и
закреплённый отпечаток не меняются. Ключ лежит в каталоге службы.

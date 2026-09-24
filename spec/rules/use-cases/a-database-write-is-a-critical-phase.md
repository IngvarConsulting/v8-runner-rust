---
id: INV.USE-CASES.A-DATABASE-WRITE-IS-A-CRITICAL-PHASE
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/290
---

# Запись в базу — критическая фаза

Шаг, который меняет базу, объявляет класс прерывания `CriticalNonAbortable`: загрузка и
применение конфигурации, изменение состава и свойств расширений, создание базы, загрузка
базы целиком из файла. Снятый посреди записи процесс оставляет базу в состоянии, которое не
назовёт никто.

Сегодня `/RestoreIB` Конфигуратора объявляет `GracefulThenKill`.

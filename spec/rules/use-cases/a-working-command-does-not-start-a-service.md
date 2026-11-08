---
id: INV.USE-CASES.A-WORKING-COMMAND-DOES-NOT-START-A-SERVICE
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/536
---

# Рабочая команда службу не поднимает

Команда, которой нужно открыть управляемую базу, при неподнятой службе отказывает до запуска платформы и
называет следующую команду. Команды, которые базу не открывают, — `make`, `convert` без базы, `status` без
`--deep`, — от службы не зависят. Совет не ведёт в новый отказ: у ещё не созданной базы он называет
`infobase create`.

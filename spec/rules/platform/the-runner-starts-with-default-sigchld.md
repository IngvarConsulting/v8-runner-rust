---
id: INV.PLATFORM.THE-RUNNER-STARTS-WITH-DEFAULT-SIGCHLD
check: [tests/cli_syntax.rs::a_designer_exit_code_survives_a_parent_that_ignores_sigchld]
---

# Раннер начинает со `SIGCHLD` по умолчанию

На Unix раннер первым шагом, до запуска потоков и потомков, возвращает `SIGCHLD` к действию
по умолчанию. Игнорирование, унаследованное от родителя через exec, не отнимает у раннера
ожидание своих процессов: процесс, вышедший сам, отвечает своим кодом выхода, а не отказом
наблюдения. Подтверждение конца снятого процесса, которого подобрал кто-то другой, — предмет
[INV.USE-CASES.AN-INTERRUPTION-STATUS-MEANS-A-TERMINAL-OUTCOME](../use-cases/an-interruption-status-means-a-terminal-outcome.md).

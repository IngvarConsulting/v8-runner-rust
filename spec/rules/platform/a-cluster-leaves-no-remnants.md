---
id: INV.PLATFORM.A-CLUSTER-LEAVES-NO-REMNANTS
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/538
---

# После кластера не остаётся его процессов

Упавший `ragent` оставляет `rmngr` и `rphost` со своими портами, а `taskkill /T` по мёртвому родителю их не снимает.
`cluster stop` и `cluster start` снимают по pid только процесс, который совпал во всём: родитель из записи мёртв,
`ParentProcessId` равен его pid, имя образа `rmngr` или `rphost`, процесс начался не раньше записанного старта `ragent`,
а хеш `machine_id` записи совпадает с машиной. Занятость `regport` таким остатком не считается чужой и к `--new-regport`
не ведёт.

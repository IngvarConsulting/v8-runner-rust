---
id: INV.CLI.AN-OLD-KEY-MAPS-TO-ITS-NEW-MEANING-FOR-ONE-CYCLE
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/191
---

# Прежний ключ без пары отображается в новый смысл на один цикл

Ключи, у которых нет пары с новым именем, принимаются один цикл выпуска так:
`--source-set <имя>` — синоним позиционного набора; `--state working` — то же, что без
ключа; `--state database` — `--state db`; `--mode incremental` и `--mode partial` — без
ключа, режим решает память.

`--mode full` не отображается, а отказывает и называет `pull --force`: молчаливое
отображение обошло бы сторожа невосстановимой работы.

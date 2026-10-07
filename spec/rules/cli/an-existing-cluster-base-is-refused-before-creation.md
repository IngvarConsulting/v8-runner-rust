---
id: INV.CLI.AN-EXISTING-CLUSTER-BASE-IS-REFUSED-BEFORE-CREATION
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/213
---

# Существующая база в кластере — отказ до создания, и превью его называет

`infobase create` в кластере до запуска Конфигуратора спрашивает кластер, зарегистрирована ли
база с этим именем, и на зарегистрированной отказывает; превью различает «будет создана» и
«уже есть». Код выхода `CREATEINFOBASE` этого не различает: «уже существует» и любой другой
отказ отвечают одним кодом, а прозу платформы раннер не читает. Вопрос задаёт `rac`
(`infobase summary list` или `infobase info --name`) через сервер администрирования
кластера.

Источник: замер [#181](../../../references/1c/confirmed-runtime-measurements.md).

---
id: DEC.2026-09-14.A-PROVIDER-IS-NAMED-BY-WHO-EXECUTES
status: active
governs: product
realized: tests/cli_infobase.rs::configuration_cf_dry_run_selects_provider_without_process_or_filesystem_mutation
supersedes: []
superseded-by: null
establishes:
  - CTR.WIRE.INFOBASE-CONFIGURATION-EXPORT-DATA
  - CTR.WIRE.INFOBASE-DUMP-DATA
  - CTR.WIRE.INFOBASE-RESTORE-DATA
changes:
  - CTR.WIRE.INFOBASE-CONFIGURATION-EXPORT-DATA
  - CTR.WIRE.INFOBASE-DUMP-DATA
  - CTR.WIRE.INFOBASE-RESTORE-DATA
---

# Имя провайдера называет исполнителя, а не способ запуска

**Решение.** Провайдер в ответе называется одним словом — тем, кто исполняет:
`designer`, `ibcmd`, и дальше `agent`, `ibcmd-rs`, `webinst`. Способ запуска в имя
не входит: `designer-batch` становится `designer`, `ibcmd-process` — `ibcmd`. Имя
опубликовано в трёх формах ответа команд `infobase.*`, и смена имени — смена этих
форм: их `version` поднимается вместе с ней.

**Почему.** Словарь провайдеров уже принят решением
[`DEC.2026-09-14.PROVIDER-CHOSEN-PER-OPERATION`](2026-09-14-provider-chosen-per-operation.md)
именно в таком виде, но в коде остались имена прежнего поколения. Пока матрица не
построена, расхождение выглядело безобидным — до тех пор, пока форма ответа не стала
закреплённой: потребитель, написанный по опубликованной форме, привязался бы к имени,
которое мы уже договорились не использовать, и платил бы за нашу правку дважды.

Суффиксы называли не исполнителя, а механику его запуска. У Конфигуратора появится
агентский режим, у `ibcmd` — и процесс, и внутрипроцессная форма; в обоих случаях
исполнитель тот же, и разводить их именами значит объявлять развилкой то, что следует
из вида цели.

**Цена.** Значения на проводе меняются, и потребитель, читавший `designer-batch`,
увидит `designer`. Совместимость здесь не удерживается: имена опубликованы вместе с
формами в этом же изменении и другого поколения потребителей не имели.

**Не затрагивает.** Матрицу `(операция, цель) → провайдеры`: она остаётся работой по
плану, здесь только словарь.

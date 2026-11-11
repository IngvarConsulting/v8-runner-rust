---
id: INV.USE-CASES.PROVIDER-DEFAULTS-LIVE-IN-CODE
check:
  - src/domain/capability.rs::ibcmd_snapshots_are_implemented_but_require_an_explicit_key
  - src/domain/capability.rs::an_experimental_provider_never_leads_a_default_chain
  - tests/cli_infobase.rs::a_default_chain_skips_the_missing_designer_and_selects_ibcmd
---

# Цепочки умолчаний живут в коде и версионируются с ним

Цепочка исполнителей по умолчанию — данные `src/domain/capability.rs`, а не настройка.
Раннер берёт первого готового в этом окружении и называет пропущенных в квитанции полем
`skipped`.

Исполнитель, помеченный экспериментальным, в цепочку умолчаний не входит ни у одного вида
цели: его выбирают только явным ключом. Улика строки — `documented`, `argv_tested`,
`live_verified` — объясняет, чем подтверждена строка, и воротами не является.

Реализованный исполнитель тоже может требовать явного ключа: `ibcmd` у DT файловой базы
помечен так в самой строке матрицы, а цепочка умолчаний остаётся `agent` → `designer`.

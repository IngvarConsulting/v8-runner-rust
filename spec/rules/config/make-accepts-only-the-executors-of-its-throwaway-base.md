---
id: INV.CONFIG.MAKE-ACCEPTS-ONLY-THE-EXECUTORS-OF-ITS-THROWAWAY-BASE
check:
  - src/config/validate.rs::make_accepts_only_the_executors_of_its_throwaway_base
  - tests/cli_agent_scenarios.rs::make_through_the_agent_is_refused_with_the_download_step
---

# `providers.make` принимает только исполнителей временной базы

Строка `make` матрицы от вида базы проекта не зависит: `ibcmd`, затем Конфигуратор.
`providers.make` принимает `ibcmd` и `designer`. `agent` снят — агент работает только с
базой проекта: валидация отказывает и называет выход `next` — `download`, который выгружает
пакет базы. `ibcmd-rs` отказывает как исполнитель без строки
(`INV.USE-CASES.IBCMD-RS-BUILDS-A-MAKE-PACKAGE-WHEN-CHOSEN`). Ни тот, ни другой отказ не
называет вид базы.

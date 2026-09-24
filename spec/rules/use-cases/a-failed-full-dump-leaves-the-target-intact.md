---
id: INV.USE-CASES.A-FAILED-FULL-DUMP-LEAVES-THE-TARGET-INTACT
check:
  - src/use_cases/dump_config.rs::dump_full_preserves_old_dump_on_platform_failure
  - src/use_cases/dump_config.rs::ibcmd_dump_full_preserves_old_target_on_platform_failure
  - src/use_cases/dump_config.rs::ibcmd_dump_full_uses_staging_dir_and_atomic_publish
---

# Неудавшаяся полная выгрузка не трогает прежнюю цель

Пока исполнитель полной выгрузки не справился, прежняя цель остаётся нетронутой:
Конфигуратор и `ibcmd` выгружают в промежуточный каталог, а не в цель.

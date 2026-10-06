---
id: INV.USE-CASES.A-THROWAWAY-BASE-SERVES-ONE-RUN-AND-IS-REMOVED
check:
  - src/use_cases/artifacts.rs::one_base_serves_every_package_of_a_run
  - src/use_cases/artifacts.rs::execute_removes_its_throwaway_base
  - src/use_cases/artifacts.rs::a_cancellation_during_creation_stops_before_the_load_and_the_base_is_removed
  - tests/cli_make_download_all.rs::make_without_a_set_builds_every_package_in_one_throwaway_base
  - src/use_cases/throwaway_infobase.rs::orphan_cleanup_removes_only_stale_own_throwaway_bases
---

# Временная база служит одному прогону и убирается

`make <SET>` создаёт свою временную базу, а обход `make` без набора — одну на все наборы:
основная конфигурация попадает в неё один раз, расширения ложатся поверх. После прогона —
удачного, отказавшего или отменённого — база убирается. Рядом с базой лежит описание её
вида, и базу, брошенную оборванным прогоном, уборка узнаёт как свою
(`INV.USE-CASES.CLEANUP-TOUCHES-ONLY-ITS-OWN-ARTEFACTS`). Замка базы и метки владельца у
временной базы нет: это не база проекта; замок цели `--output` остаётся.

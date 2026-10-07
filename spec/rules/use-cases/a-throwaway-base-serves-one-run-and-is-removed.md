---
id: INV.USE-CASES.A-THROWAWAY-BASE-SERVES-ONE-RUN-AND-IS-REMOVED
check:
  - src/use_cases/artifacts.rs::one_base_serves_every_package_of_a_run
  - src/use_cases/artifacts.rs::execute_removes_its_throwaway_base
  - src/use_cases/artifacts.rs::a_cancellation_during_creation_stops_before_the_load_and_the_base_is_removed
  - tests/cli_make_download_all.rs::make_without_a_set_builds_every_package_in_one_throwaway_base
  - src/use_cases/throwaway_infobase.rs::orphan_cleanup_removes_only_stale_own_throwaway_bases
  - src/use_cases/throwaway_infobase.rs::a_failed_creation_names_the_failed_orphan_cleanup
  - src/use_cases/artifacts.rs::a_failed_run_removes_its_throwaway_base
  - src/use_cases/artifacts.rs::a_designer_walk_builds_externals_in_its_own_base
  - src/use_cases/throwaway_infobase.rs::a_stale_base_that_cannot_be_removed_does_not_stop_the_build
---

# Временная база служит одному прогону и убирается

`make <SET>` создаёт свою временную базу, а обход `make` без набора — одну на все наборы у
каждого исполнителя: основная конфигурация попадает в базу Конфигуратора один раз,
расширения и внешние обработки ложатся поверх. После прогона — удачного, отказавшего или
отменённого — база убирается. Рядом с базой лежит описание её вида, и базу, брошенную
оборванным прогоном, уборка узнаёт как свою
(`INV.USE-CASES.CLEANUP-TOUCHES-ONLY-ITS-OWN-ARTEFACTS`). Брошенная база, которую убрать не
удалось, прогон не останавливает: ответ называет неудачу предупреждением, а если своя база
не создалась — отказ создания. Замка базы и метки владельца у
временной базы нет: это не база проекта; замок цели `--output` остаётся.

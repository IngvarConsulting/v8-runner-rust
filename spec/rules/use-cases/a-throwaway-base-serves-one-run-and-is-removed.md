---
id: INV.USE-CASES.A-THROWAWAY-BASE-SERVES-ONE-RUN-AND-IS-REMOVED
check:
  - src/use_cases/throwaway_infobase.rs::orphan_cleanup_removes_only_stale_own_throwaway_bases
  - src/use_cases/throwaway_infobase.rs::a_failed_creation_names_the_failed_orphan_cleanup
  - src/use_cases/throwaway_infobase.rs::a_stale_base_that_cannot_be_removed_does_not_stop_the_build
  - tests/cli_convert.rs::convert_a_failed_ibcmd_step_removes_the_base_and_keeps_the_receipt
  - tests/cli_convert.rs::convert_a_cancelled_run_removes_the_throwaway_base
---

# Временная база разбора пакета служит одному прогону

`convert <FILE> --to xml` разбирает пакет во временной базе под
`workPath/temp/throwaway-infobases/`. После успеха, отказа или отмены раннер
удаляет её. Уборка опознаёт брошенные базы по имени и метаданным. Неудачную
уборку команда сообщает в ответе. База сборки `make` и `convert --to package`
живёт отдельно.

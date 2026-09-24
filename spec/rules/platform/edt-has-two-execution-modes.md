---
id: INV.PLATFORM.EDT-HAS-TWO-EXECUTION-MODES
check:
  - src/use_cases/init_project.rs::init_uses_one_shot_edt_when_interactive_mode_is_disabled
  - src/platform/edt.rs::shared_session_drop_does_not_shutdown_other_manager_owner
  - src/platform/edt_session.rs::baseline_reset_runs_before_each_user_command
  - tests/mcp_stdio.rs::mcp_stdio_edt_syntax_resets_interactive_state_before_each_call
  - src/platform/edt_session.rs::baseline_probe_mismatch_restarts_and_drains_queue
  - src/platform/edt_session.rs::fatal_baseline_timeout_under_internal_cap_restarts_and_drains_queue
  - src/platform/edt_session.rs::shutdown_drains_queued_requests
  - src/platform/edt_session.rs::budget_exhausted_during_baseline_returns_queued_timeout_without_restart
---

# У EDT ровно два режима исполнения

EDT CLI исполняет команды либо отдельным процессом на каждый вызов, либо одной общей живой
сессией с очередью, сбросом базового состояния перед пользовательской командой,
перезапуском с дренажом очереди, когда сброс не удался, и дренажом при завершении. Предел
шага, исчерпанный во время сброса, даёт таймаут в очереди и сессию не перезапускает.
Третьего режима — интерактивного, но не общего — нет.

Время жизни общей сессии назначает тот, кто её держит, и режимом оно не является.

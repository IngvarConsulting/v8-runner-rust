---
id: INV.PLATFORM.EDT-HAS-TWO-EXECUTION-MODES
check:
  - src/use_cases/init_project.rs::init_uses_one_shot_edt_when_interactive_mode_is_disabled
  - src/platform/edt.rs::shared_session_drop_does_not_shutdown_other_manager_owner
---

# У EDT ровно два режима исполнения

EDT CLI исполняет команды либо отдельным процессом на каждый вызов, либо одной общей живой
сессией с очередью, сбросом базового состояния перед пользовательской командой,
перезапуском и дренажом при завершении. Третьего режима — интерактивного, но не общего —
нет.

Время жизни общей сессии назначает тот, кто её держит, и режимом оно не является.

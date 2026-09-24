---
id: INV.PLATFORM.ONLY-A-LONG-LIVED-HOST-PREWARMS-EDT
check:
  - src/use_cases/init_project.rs::init_cli_interactive_auto_start_remains_lazy_without_edt_commands
  - src/use_cases/init_project.rs::init_cli_shared_session_does_not_charge_startup_against_first_command_timeout
---

# Прогревает общую сессию EDT только долгоживущий хозяин

Заблаговременный прогрев включён лишь там, где процесс живёт между вызовами, — у
MCP-сервера. Короткая команда CLI поднимает сессию лениво, при первом обращении к EDT, и
держит её только в своих границах: `tools.edt_cli.auto_start` прогрева у неё не включает.

---
id: INV.PLATFORM.STANDALONE-PORTS-AND-KEY-ARE-PINNED
check:
  - src/use_cases/managed_stand/standalone.rs::the_ports_and_the_key_stay_from_start_to_start
  - src/use_cases/managed_stand/standalone.rs::a_taken_pinned_block_moves_the_server_to_a_new_block_with_the_same_key
  - src/use_cases/managed_stand/standalone.rs::a_new_block_keeps_clear_of_the_port_of_the_runners_postgres
  - src/use_cases/managed_stand/standalone.rs::a_taken_block_without_the_process_list_is_refused_and_the_ports_stay
  - src/use_cases/managed_stand/standalone.rs::a_foreign_server_of_this_base_on_the_pinned_block_is_refused_and_the_ports_stay
  - src/use_cases/managed_stand/standalone.rs::a_foreign_server_over_the_same_base_is_named_and_left_alone
  - src/use_cases/managed_stand/standalone.rs::no_free_block_is_refused_and_the_ports_stay
  - src/use_cases/managed_stand/standalone.rs::restart_moves_a_running_server_off_a_taken_port
  - src/use_cases/managed_stand/standalone.rs::a_failed_start_on_a_new_block_keeps_the_previous_block
  - src/use_cases/managed_stand/standalone.rs::a_failed_start_over_a_stale_record_keeps_the_previous_block
  - src/use_cases/managed_stand/standalone.rs::a_pinned_block_over_the_port_of_the_runners_postgres_moves
  - src/use_cases/managed_stand/standalone.rs::the_refusals_of_a_taken_block_come_before_the_postgres_starts
---

# Ключ автономного сервера закреплён, а занятый блок портов сменяется

Блок портов и ключ хоста SSH-шлюза создаются при первом запуске и остаются от запуска к запуску. Ключ лежит в
каталоге службы и не меняется никогда: закреплённый отпечаток остаётся верным.

Если порт закреплённого блока занят чужим процессом или в блок попал закреплённый порт PostgreSQL раннера,
`standalone start` и `restart` берут новый свободный блок тем же перебором, что и первый запуск: он не задевает прежний
блок и порт PostgreSQL раннера. Новый блок закрепляется, когда
сервер на нём поднялся; неудавшийся запуск возвращает прежний блок. Чужой процесс остаётся работать. Ответ называет новые `connection`, `gate` и `http`, прямой порт прежнего блока в
`previous_port` и предупреждение с портом и pid того, кто его занял, или с портом PostgreSQL раннера.

Адрес стенда живёт только в его состоянии под `localPath`. Файлы `v8project*.yaml` раннер при смене блока не правит, и
адрес у `origin` не пишется: адрес берут из ответа `start` или `status`, а не кешируют.

Блок остаётся прежним, а запуск отказывает `workspace_busy`, если над этой же базой работает чужой `ibsrv` или если
перечня процессов нет и не сказать, кто держит порт; эти отказы приходят до запуска PostgreSQL шагом. Нет свободного
блока — отказ среды; закреплённое не меняется.

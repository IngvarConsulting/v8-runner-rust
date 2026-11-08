---
id: INV.USE-CASES.A-WORK-PATH-INSIDE-A-SET-IS-NOT-A-SOURCE
check:
  - src/change_detection/scanner.rs::the_build_directory_is_not_a_source_below_the_root
  - src/change_detection/scanner.rs::a_work_path_inside_the_root_is_not_a_source_whatever_its_name
  - src/change_detection/analyzer.rs::a_service_file_in_a_work_path_inside_the_set_is_not_a_change
  - src/use_cases/exchange_guard.rs::a_created_base_does_not_remember_a_work_path_inside_its_set
  - src/use_cases/agent_session.rs::a_root_copied_for_the_agent_leaves_the_agent_directory_behind
  - src/use_cases/artifacts.rs::ibcmd_build_expands_objects_with_the_build_scan_scope
---

# `workPath` внутри набора не исходник

Набор может лежать в корне рабочей области, и тогда `workPath` и служебный `.build` оказываются
внутри него. Определение изменений их не читает: файл там не становится правкой, а чужая
блокировка на нём не прерывает анализ. `workPath` исключается как каталог, с любым именем, а
`.build` по имени, как остальные порождённые каталоги
(`INV.USE-CASES.A-SELECTED-ROOT-IS-SCANNED-WHATEVER-ITS-NAME`).

Там, где символическую ссылку создать нельзя, агент получает копию набора. Если его каталог лежит
внутри этого набора, копия не входит в собственное назначение и не несёт `workPath` и порождённых
каталогов верхнего уровня. Глубже такое имя принадлежит метаданным, и они копируются.

Память, записанная прежней версией, может называть файлы этих каталогов. Следующая отправка видит
их удалёнными и выполняет полную загрузку; потом память чиста.

Сборка пакета исключает вложенный `workPath` и из снимка, и из списка частичной
загрузки. Каталог самой базы сборки остаётся зарезервированным.

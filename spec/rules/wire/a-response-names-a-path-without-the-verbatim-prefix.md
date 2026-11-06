---
id: INV.WIRE.A-RESPONSE-NAMES-A-PATH-WITHOUT-THE-VERBATIM-PREFIX
check:
  - tests/cli_infobase_cross_platform.rs::a_download_answers_with_an_ordinary_path
  - tests/cli_infobase_cross_platform.rs::an_owner_warning_names_the_copies_with_ordinary_paths
  - tests/cli_external_full_pull.rs::accepted_external_full_pull_remembers_the_original_source_for_the_next_push
  - src/mcp/server.rs::an_mcp_answer_names_paths_without_the_verbatim_prefix
  - src/support/path.rs::a_response_on_windows_names_paths_without_the_verbatim_prefix
  - src/support/path.rs::verbatim_prefixes_leave_text_before_a_drive_and_at_a_unc_name
  - src/support/path.rs::a_response_off_windows_keeps_a_file_name_spelled_like_the_prefix
  - tests/architecture_guardrails.rs::a_response_reaches_stdout_only_through_its_owners
---

# Ответ называет путь без приставки `\\?\`

На Windows ответ команды называет путь обычным написанием: `E:\proj\main.cf` и
`\\srv\share\main.cf`, а не `\\?\E:\proj\main.cf` и `\\?\UNC\srv\share\main.cf`. Это касается
всех полей и текстов ответа: JSON командной строки, текстовой ленты, ответа и внутренней
ошибки MCP, кадров согласования внешней публикации. `\\?\Volume{…}\` остаётся как есть: другого
написания у него нет. Вне Windows ответ не меняется: там такая запись — допустимое имя файла.

Канонический путь внутри раннера сохраняет приставку, потому что на его тексте стоят
идентичность цели, имена замков и привязка хеш-памяти. По той же причине её сохраняют метка
владельца базы и файлы замков: это не ответ.

Операнды публикации остаются как есть: `target_path` и `stage_path` кадра `ready`
([форма](full-pull-publication.md)). Вызывающий публикует по этим путям, а без приставки путь
длиннее 260 знаков не открылся бы.

Вне этого обязательства остаётся то, что не является ответом команды: журнал действий и
сообщения в stderr до начала работы команды и при запуске сервера MCP.

**Принятый риск.** Показанный путь длиннее 260 знаков или с именем, которое без приставки
значит другое (`con.cf`, `out.`), внешняя программа может не открыть. Проход идёт по тексту и
не видит конца пути, поэтому такие случаи не различает. Владелец принял этот риск, предпочтя
обычные пути во всех ответах.

---
id: INV.USE-CASES.A-PROJECT-SHAPE-NARROWS-A-DEFAULT-CHAIN
check:
  - src/domain/capability.rs::the_project_shape_drops_the_agent_where_it_has_no_adapter
  - tests/cli_tools_download.rs::tools_download_client_mcp_artifact_follows_the_push_chain_shaped_by_the_tool_extension
  - tests/contract_receipt.rs::an_edt_project_pushes_through_the_designer_unless_a_key_names_the_agent
---

# Форма проекта сужает цепочку умолчаний

У файловой базы и кластера агент выпадает из цепочки `push` и `pull` проекта с исходниками
EDT и из цепочки `push` проекта, где объявлено расширение-инструмент
(`tools.client_mcp.extension`): адаптера для них у агента нет. Остальные строки форма
проекта не трогает, а у автономного сервера цепочка не сужается. Ключ `providers.*`
форма проекта не переписывает.

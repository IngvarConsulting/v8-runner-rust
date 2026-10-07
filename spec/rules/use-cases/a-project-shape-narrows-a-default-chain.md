---
id: INV.USE-CASES.A-PROJECT-SHAPE-NARROWS-A-DEFAULT-CHAIN
check:
  - src/domain/capability.rs::the_project_shape_drops_the_agent_where_it_has_no_adapter
  - src/config/validate.rs::validates_client_mcp_extension_source_and_artifact_contract
---

# Форма проекта сужает цепочку умолчаний

У файловой базы и кластера агент выпадает из цепочки `push` и `pull` проекта с исходниками
EDT и из цепочки `push` проекта, где объявлено расширение-инструмент
(`tools.client_mcp.extension`): адаптера для них у агента нет. Остальные строки форма
проекта не трогает, а у автономного сервера цепочка не сужается. Ключ `providers.*`
форма проекта не переписывает.

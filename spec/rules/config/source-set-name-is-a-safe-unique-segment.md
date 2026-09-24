---
id: INV.CONFIG.SOURCE-SET-NAME-IS-A-SAFE-UNIQUE-SEGMENT
check:
  - src/config/validate.rs::rejects_source_set_name_with_path_separator
  - src/config/validate.rs::rejects_source_set_name_with_parent_traversal
  - src/config/validate.rs::accepts_safe_source_set_name
---

# Имя набора уникально и безопасно как сегмент пути

Повторяющееся имя, имя с разделителями пути и совпадение разрешённых путей после нормализации отклоняются.

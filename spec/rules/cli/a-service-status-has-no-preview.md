---
id: INV.CLI.A-SERVICE-STATUS-HAS-NO-PREVIEW
check:
  - tests/cli_dbms.rs::status_has_no_preview
  - tests/cli_cluster_managed.rs::status_has_no_preview
---

# У `status` служб нет превью

`status` у `dbms`, `cluster` и `standalone` ничего не запускает, поэтому `--dry-run` у него отвергается.

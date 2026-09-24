---
id: INV.USE-CASES.PUBLICATION-IS-NEVER-A-DEFAULT-STEP
check: [tests/provider_matrix.rs::no_default_chain_names_the_publication_provider]
---

# Публикация не бывает попутным шагом

Ни `push`, ни `test`, ни любая другая команда не публикует базу попутно и не чинит
сломанную публикацию: имени исполнителя публикации нет ни в одной цепочке умолчаний.

Публикация — отдельное намерение и делается командой `publish`. Сценарии, которым она
нужна, называют её шагом явно.

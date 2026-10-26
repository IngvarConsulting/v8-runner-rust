---
id: INV.USE-CASES.A-PROJECTION-ACTS-AS-THE-WORKING-COPY-IT-NAMES
check:
  - tests/cli_infobase_owner.rs::a_projection_acting_as_the_owner_hears_no_warning_and_keeps_the_owner
  - tests/cli_infobase_owner.rs::a_projection_of_another_copy_is_warned_like_that_copy
  - tests/cli_infobase_owner.rs::a_working_copy_that_does_not_declare_the_base_is_refused
  - tests/cli_infobase_owner.rs::without_the_key_a_projection_is_another_copy
  - tests/cli_infobase_owner.rs::the_first_write_of_a_projection_records_the_copy_it_names
  - tests/cli_infobase_owner.rs::a_projection_recorded_before_the_key_gives_way_to_the_copy_it_names
  - tests/cli_infobase_owner.rs::a_working_copy_in_the_project_file_is_refused
  - tests/cli_infobase_owner.rs::a_relative_working_copy_is_resolved_from_the_local_layer
---

# Проекция выступает рабочей копией, которую называет

Рабочую копию для метки владельца файловой базы
(`INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`) называет каталог проекта.
Каталог, собранный из копии, — проекция, которую вызывающий запускает вместо неё, —
называет её ключом `working-copy` своего местного слоя и для метки выступает этой копией:
на её базе пишет без предупреждения, на базе другой копии получает то же предупреждение,
что и она, и записывается в метку ею. Запись самой проекции, сделанная до ключа, уступает
названной копии, как запись ушедшего владельца. Остальное — исходники, `workPath`, память,
в том числе о взятии базы, — остаётся своим. У базы без метки — серверной — ключ не
действует.

Назвать можно только живую копию этой же базы: её каталог есть и объявляет эту базу.
Иначе команда отказывает до платформы, и метка не меняется: так нельзя назваться чужой
копией и перехватить её базу. В проектном файле ключ не принимается: какой копией выступает
каталог, знает машина, а не проект. Без ключа копия — каталог проекта, как прежде.

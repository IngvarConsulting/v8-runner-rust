---
id: INV.PLATFORM.A-MISSING-COMPONENT-NAMES-WHAT-IS-INSTALLED
status: active
governs: product
decision: DEC.2026-09-16.A-MISSING-COMPONENT-IS-NAMED-WITH-ITS-INSTALLATIONS
check: [src/platform/locator.rs::a_missing_component_is_named_together_with_the_installations_that_have_it, src/platform/locator.rs::a_version_without_any_installation_names_where_the_component_lives, src/platform/locator.rs::a_component_absent_everywhere_is_named_as_absent_everywhere, src/platform/locator.rs::an_empty_inventory_names_the_roots_it_searched, src/platform/locator.rs::an_explicit_file_path_is_inventoried_as_the_installation_around_it]
scope: [platform]
---

# Ненайденная утилита платформы названа вместе с описью установок

Отказ по утилите платформы называет опись, а не одно имя файла. Установки нашлись —
отказ называет недостающий компонент и версии: и те, что подошли под маску, и те, где
компонент есть, а если его нет нигде, говорит и это. Не нашлось ни одной — отказ
называет места, в которых искал. Опись строится по тому же пути, по которому шёл поиск:
про явно указанный `path` отказ не вправе сказать, что установки там нет.

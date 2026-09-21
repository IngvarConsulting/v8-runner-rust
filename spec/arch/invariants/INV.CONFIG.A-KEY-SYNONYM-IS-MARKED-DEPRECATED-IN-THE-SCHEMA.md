---
id: INV.CONFIG.A-KEY-SYNONYM-IS-MARKED-DEPRECATED-IN-THE-SCHEMA
status: planned
governs: product
decision: DEC.2026-09-21.OLD-NAMES-ARE-HIDDEN-SYNONYMS-FOR-ONE-CYCLE
check: null
scope: [config, docs]
---

# Прежний ключ конфигурации помечен в схеме устаревшим

Каждый ключ, который модель принимает как синоним, присутствует в опубликованной схеме с
`deprecated: true`; ключ, принимаемый моделью и отсутствующий в схеме, — нарушение.

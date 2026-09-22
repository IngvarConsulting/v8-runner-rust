---
id: DEC.2026-09-22.MAINTENANCE-RELEASE-ORIGIN
status: active
governs: product
realized: tests/release_governance.py
supersedes: []
superseded-by: null
establishes: [INV.RELEASE.MAINTENANCE-ORIGIN]
---

# Совместимый выпуск 0.11 имеет отдельный защищённый источник

**Решение.** Выпуск исправлений 0.11 допускается из защищённой release/0.11,
отделённой от следующего CLI в master. Используется единственный существующий
release workflow и все его проверки аудита и происхождения. Закрытый список
источников: master и release/0.11; maintenance допускает только версии 0.11.x.

HEAD, тег, удалённая выбранная ветка и SHA запуска должны совпасть. Обе
проверки attestation привязаны к тому же выбранному ref. Защиты maintenance
ветки не слабее master; environment release не ослабляется. Исторические
master-выпуски сохраняют свою проверку происхождения. Локальная публикация
бинарников и обход workflow не являются способом maintenance-выпуска.

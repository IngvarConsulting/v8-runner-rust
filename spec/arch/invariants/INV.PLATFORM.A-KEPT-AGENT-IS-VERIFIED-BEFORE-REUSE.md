---
id: INV.PLATFORM.A-KEPT-AGENT-IS-VERIFIED-BEFORE-REUSE
status: planned
governs: product
decision: DEC.2026-09-15.A-KEPT-AGENT-LIVES-WITH-THE-WORKSPACE
check: null
scope: [platform, use-cases]
---

# Живой агент проверяется перед каждым использованием

Команда использует агента из удостоверения `<workPath>/agent/agent.json` только после
того, как убедилась: процесс с записанным pid жив, личность (строка соединения,
платформа и её версия, `AgentBaseDir`) совпадает с текущим конфигом, SSH-сессия
аутентифицирована. Любое расхождение — остановка старого процесса и запуск нового, с
пометкой в квитанции; удостоверение без процесса удаляется.

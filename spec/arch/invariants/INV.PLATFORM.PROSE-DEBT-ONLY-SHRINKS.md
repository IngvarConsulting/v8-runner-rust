---
id: INV.PLATFORM.PROSE-DEBT-ONLY-SHRINKS
status: active
governs: product
decision: DEC.2026-09-12.TOOL-PROSE-NEVER-DECIDES
check: tests/tool_output_contract.rs::tool_prose_never_decides_and_the_declared_debt_only_shrinks
scope: [platform, use-cases]
---

# Реестр решений по прозе может только сокращаться

Страж разбирает продуктовый код и находит места, где решение принимается по тексту ответа инструмента. Объявленный долг может только уменьшаться: новое место валит проверку, исчезнувшее — тоже, чтобы реестр не превращался в историю.

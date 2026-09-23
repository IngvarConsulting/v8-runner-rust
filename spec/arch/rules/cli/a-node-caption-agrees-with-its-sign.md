---
id: INV.CLI.A-NODE-CAPTION-AGREES-WITH-ITS-SIGN
check: [tests/architecture_guardrails.rs::a_renderer_never_spells_the_outcome_word_itself]
---

# Подпись узла согласована с его знаком

Слово исхода в подписи узла выбирает presenter — тем же значением, которым выбирает знак. Рендерер называет предмет; произносить исход сам он не вправе, иначе подпись и знак расходятся.

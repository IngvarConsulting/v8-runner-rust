---
id: INV.USE-CASES.A-GIT-WARNING-DOES-NOT-CANCEL-ITS-ANSWER
check:
  - src/platform/git.rs::a_warning_on_a_successful_status_keeps_the_answer
  - src/platform/git.rs::an_unreadable_subdirectory_is_unknown_not_clean
---

# Предупреждение гита не отменяет его ответ

Сторож замены читает ответ гита по коду выхода и по самому каталогу, а не по тексту гита.
Удачный `git status` остаётся ответом и с предупреждением в stderr: предупреждение, например
о замене концов строк, уходит в журнал. Неполный перечень ответом не считается: когда при
удачном выходе stderr непуст, раннер обходит каталог, и подкаталог, который не прочесть,
даёт «ответа нет».

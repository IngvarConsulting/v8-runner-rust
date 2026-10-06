---
id: INV.USE-CASES.A-VERSION-FILE-ALONE-IS-DUMPED-ONLY-WHEN-THE-DIRECTORY-MATCHES-THE-BASE
check:
  - tests/cli_push_generation.rs::a_lost_version_file_is_dumped_alone_after_a_full_push
  - tests/cli_push_generation.rs::a_version_file_dumped_while_the_base_changed_is_removed
  - tests/cli_push_generation.rs::without_an_answer_the_version_file_is_not_dumped_alone
---

# Один файл версий выгружается, только когда каталог совпадает с базой

Платформа умеет выгрузить один файл версий, не трогая остального (`-configDumpInfoOnly`).
Такой файл описывает базу такой, какая она сейчас, и заявляет, что все её объекты уже лежат
в каталоге. Раннер выгружает его для набора только при доказанном совпадении каталога этого
набора и базы: в той же команде и под тем же замком, сразу после удачного полного `push`
или полного `pull` этого набора, либо после частичного `push`, перед которым поколение
набора совпало с записанным. Поколение до и после операции сравнивается внутри одного
инструмента (`INV.USE-CASES.A-GENERATION-TOKEN-IS-COMPARED-WITHIN-ITS-OWN-TOOL`) и не
изменилось; у расширения сравнивается его собственное поколение. Отсутствие ответа о
поколении доказательством не является. Так раннер восстанавливает потерянный файл версий
без полной выгрузки. Один файл версий выгружает Конфигуратор; полная выгрузка любым
исполнителем кладёт файл версий сама. Если поколение во время выгрузки файла изменилось или
ответа нет, выгруженный файл убирается. В остальных случаях действует
`INV.USE-CASES.A-MISSING-VERSION-FILE-IS-KNOWN-BEFORE-THE-PLATFORM-STARTS`.

Источник: [`platform.html#t21`](../../../docs/site/platform.html#t21),
[`sources.html#runner`](../../../docs/site/sources.html#runner).

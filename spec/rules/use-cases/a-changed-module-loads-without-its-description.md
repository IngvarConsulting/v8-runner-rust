---
id: INV.USE-CASES.A-CHANGED-MODULE-LOADS-WITHOUT-ITS-DESCRIPTION
check:
  - src/change_detection/partial_load.rs::a_changed_common_module_loads_without_its_description
  - src/change_detection/partial_load.rs::a_changed_form_module_loads_without_the_form_description
  - src/change_detection/partial_load.rs::a_changed_module_and_its_changed_description_load_together
  - src/change_detection/partial_load.rs::an_added_common_module_loads_with_its_description_and_the_root
  - src/change_detection/partial_load.rs::a_module_added_to_an_existing_object_loads_with_its_descriptions
  - src/change_detection/partial_load.rs::a_changed_object_module_still_loads_with_its_descriptions
  - src/change_detection/partial_load.rs::paths_that_only_look_like_a_lone_module_keep_their_descriptions
  - src/change_detection/partial_load.rs::letter_case_does_not_change_which_module_loads_alone
  - src/change_detection/partial_load.rs::the_sftp_list_takes_the_module_descriptions_back
---

# Изменённый модуль грузится без описания объекта

Частичная загрузка передаёт изменённый модуль общего модуля
(`CommonModules/<М>/Ext/Module.bsl`) и изменённый модуль формы (`…/Forms/<Ф>/Ext/Form/Module.bsl`,
`CommonForms/<Ф>/Ext/Form/Module.bsl`) одним `.bsl`. Описание объекта, описание формы,
`Form.xml` и описания объектов выше по дереву входят в список, только если изменились
сами. `-partial` загружает отдельно описание объекта без внешних свойств, внешнее свойство
и модуль формы без описания формы ([справка](../../../references/1c/designer-spec.md#команда-loadconfigfromfiles)).
Вид модуля определяется по элементу выгрузки, которому принадлежит файл, а не по виду
строки пути; регистр букв в пути на решение не влияет.

Проверено на платформе:

- Конфигуратор: загрузка одного `Module.bsl` общего модуля применяется, правка видна в
  следующей выгрузке ([замер](../../../references/1c/confirmed-runtime-measurements.md#сценарии-список-против-диска));
- агент через каталог обмена на той же машине: `.bsl` общих модулей без описаний применились
  (стенд КА, замер в PR #600).

Модуль формы одним `.bsl` не замерен ни у одного исполнителя: правило опирается на справку.
`ibcmd` получает тот же список и не замерен. Агент по SFTP — с описанием, пока не замерено:
перед передачей список возвращает описания владельцев каждого `.bsl`, как до этого правила.

Описания существующих файлов рядом с модулем и выше по дереву остаются в списке, когда:

- модуль добавлен, а его объект уже был: применение нового модуля без описаний не замерено;
- добавлен сам объект: список несёт корневое описание и все файлы объекта
  (`INV.USE-CASES.A-LOAD-IS-PARTIAL-UNLESS-NOTHING-TO-COMPARE`);
- изменён модуль другого вида — объекта, менеджера, набора записей, менеджера значения,
  команды: что платформа применяет такой модуль один, не замерено.

Модули корня (`Ext/*.bsl` рядом с `Configuration.xml`) описания объекта не имеют и
грузятся одни, как и прежде.

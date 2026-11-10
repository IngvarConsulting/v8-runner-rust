---
id: INV.USE-CASES.A-CHANGED-MODULE-LOADS-WITHOUT-ITS-DESCRIPTION
check:
  - src/change_detection/partial_load.rs::a_changed_common_module_loads_without_its_description
  - src/change_detection/partial_load.rs::a_changed_form_module_loads_without_the_form_description
  - src/change_detection/partial_load.rs::a_changed_module_and_its_changed_description_load_together
  - src/change_detection/partial_load.rs::an_added_common_module_loads_with_its_description_and_the_root
  - src/change_detection/partial_load.rs::a_module_added_to_an_existing_object_loads_with_its_descriptions
  - src/change_detection/partial_load.rs::a_changed_object_module_still_loads_with_its_descriptions
---

# Изменённый модуль грузится без описания объекта

Частичная загрузка передаёт изменённый модуль общего модуля
(`CommonModules/<М>/Ext/Module.bsl`) и изменённый модуль формы (`…/Forms/<Ф>/Ext/Form/Module.bsl`,
`CommonForms/<Ф>/Ext/Form/Module.bsl`) одним `.bsl`. Описание объекта, описание формы,
`Form.xml` и описания объектов выше по дереву входят в список, только если изменились
сами. `-partial` загружает отдельно описание объекта без внешних свойств, внешнее свойство
и модуль формы без описания формы ([справка](../../../references/1c/designer-spec.md#команда-loadconfigfromfiles)).
Загрузка одного `Module.bsl` общего модуля применяется: правка видна в следующей выгрузке
([замер](../../../references/1c/confirmed-runtime-measurements.md#сценарии-список-против-диска)).

Описания существующих файлов рядом с модулем и выше по дереву остаются в списке, когда:

- модуль добавлен, а его объект уже был: применение нового модуля без описаний не замерено;
- добавлен сам объект: список несёт корневое описание и все файлы объекта
  (`INV.USE-CASES.A-LOAD-IS-PARTIAL-UNLESS-NOTHING-TO-COMPARE`);
- изменён модуль другого вида — объекта, менеджера, набора записей, менеджера значения,
  команды: что платформа применяет такой модуль один, не замерено.

Модули корня (`Ext/*.bsl` рядом с `Configuration.xml`) описания объекта не имеют и
грузятся одни, как и прежде.

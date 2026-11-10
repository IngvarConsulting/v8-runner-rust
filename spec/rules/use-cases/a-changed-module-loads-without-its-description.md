---
id: INV.USE-CASES.A-CHANGED-MODULE-LOADS-WITHOUT-ITS-DESCRIPTION
check:
  - src/change_detection/partial_load.rs::a_changed_common_module_loads_without_its_description
  - src/change_detection/partial_load.rs::a_changed_form_module_loads_without_the_form_description
  - src/change_detection/partial_load.rs::a_changed_module_and_its_changed_description_load_together
  - src/change_detection/partial_load.rs::an_added_common_module_loads_with_its_description_and_the_root
  - src/change_detection/partial_load.rs::a_module_added_to_an_existing_object_loads_with_its_descriptions
  - src/change_detection/partial_load.rs::a_changed_object_module_loads_without_its_description
  - src/change_detection/partial_load.rs::a_changed_manager_record_set_or_command_module_still_loads_with_its_descriptions
  - src/change_detection/partial_load.rs::paths_that_only_look_like_a_lone_module_keep_their_descriptions
  - src/change_detection/partial_load.rs::letter_case_does_not_change_which_module_loads_alone
  - src/change_detection/partial_load.rs::the_sftp_list_takes_the_module_descriptions_back
---


# Изменённый модуль грузится без описания объекта

Частичная загрузка передаёт одним `.bsl` изменённый модуль общего модуля
(`CommonModules/<М>/Ext/Module.bsl`), модуль формы (`…/Forms/<Ф>/Ext/Form/Module.bsl`,
`CommonForms/<Ф>/Ext/Form/Module.bsl`) и модуль объекта (`<Вид>/<Объект>/Ext/ObjectModule.bsl`
объекта верхнего уровня). Описание объекта, описание формы, `Form.xml` и описания объектов
выше по дереву входят в список, только если изменились сами. `-partial` загружает отдельно
описание объекта без внешних свойств, внешнее свойство и модуль формы без описания формы
([справка](../../../references/1c/designer-spec.md#команда-loadconfigfromfiles)). Вид модуля
определяется по элементу выгрузки, которому принадлежит файл, а не по виду строки пути;
регистр букв в пути на решение не влияет.

Проверено на платформе: правка применилась, и `v8-runner pull` вернул её метку в исходники
([замер](../../../references/1c/confirmed-runtime-measurements.md#модуль-без-описания-объекта-599)):

- агент Конфигуратора через локальный каталог обмена: общие модули, модуль формы справочника,
  модуль общей формы, модуль объекта справочника;
- `ibcmd infobase config import files --partial`: общие модули;
- пакетный Конфигуратор: общий модуль
  ([прежний замер](../../../references/1c/confirmed-runtime-measurements.md#сценарии-список-против-диска)).

Агент по SFTP — с описанием, пока не замерено: перед передачей список возвращает описания
владельцев каждого `.bsl`, как до этого правила.

Описания существующих файлов рядом с модулем и выше по дереву остаются в списке, когда:

- модуль добавлен, а его объект уже был: применение нового модуля без описаний не замерено;
- добавлен сам объект: список несёт корневое описание и все файлы объекта
  (`INV.USE-CASES.A-LOAD-IS-PARTIAL-UNLESS-NOTHING-TO-COMPARE`);
- изменён модуль менеджера, набора записей, менеджера значения или команды: что платформа
  применяет такой модуль один, не замерено.

Модули корня (`Ext/*.bsl` рядом с `Configuration.xml`) описания объекта не имеют и
грузятся одни, как и прежде.

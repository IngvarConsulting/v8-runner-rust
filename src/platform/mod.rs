/// Designer agent driven through the system `ssh` client.
pub mod agent;
pub mod agent_keeper;
/// Opening a URL in the user's browser.
pub mod browser;
pub mod connection;
pub mod designer;
pub mod download;
/// Прогноз режима выгрузки по изменившемуся: что платформа сделает с `-update`.
pub mod dump_forecast;
/// Версия формата иерархической выгрузки в файле версий и у платформы.
pub mod dump_format;
pub mod edt;
pub mod edt_session;
pub mod enterprise;
pub mod extension_inventory;
/// Ответ платформы о поколении конфигурации.
pub mod generation;
pub mod git;
pub mod ibcmd;
/// `ibsrv` the runner starts itself.
pub mod ibsrv;
pub mod interactive;
/// The registry of live Designer agents of this OS account.
pub mod live_registry;
pub mod locator;
/// PostgreSQL build for 1C: utility arguments and data directory files.
pub mod postgres;
pub mod process;
pub mod rac;
pub mod ragent;
pub mod result;
/// Маскирование секретов в составленных аргументах.
pub mod secrets;
/// Long-lived services of the managed stand.
pub mod service;
/// The record of a service: what is running, on which ports, whose it is.
pub mod service_record;
pub mod sftp;
#[cfg(test)]
pub(crate) mod test_git;
pub mod utilities;
/// `webinst` command composition.
pub mod webinst;

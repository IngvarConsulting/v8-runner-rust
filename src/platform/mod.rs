/// Designer agent driven through the system `ssh` client.
pub mod agent;
/// Opening a URL in the user's browser.
pub mod browser;
pub mod connection;
pub mod designer;
pub mod download;
/// Версия формата иерархической выгрузки в файле версий и у платформы.
pub mod dump_format;
pub mod edt;
pub mod edt_session;
pub mod enterprise;
pub mod extension_inventory;
pub mod git;
pub mod ibcmd;
pub mod interactive;
pub mod locator;
pub mod process;
pub mod result;
/// Маскирование секретов в составленных аргументах.
pub mod secrets;
pub mod sftp;
#[cfg(test)]
pub(crate) mod test_git;
pub mod utilities;
/// `webinst` command composition.
pub mod webinst;

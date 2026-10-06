mod app;
mod change_detection;
mod cli;
mod command_data;
mod command_envelope;
mod config;
mod domain;
mod mcp;
mod output;
mod parsers;
mod platform;
mod support;
mod use_cases;

use std::process;

fn main() {
    // Первым шагом, пока поток один и потомков нет: действие сигнала общее для процесса.
    #[cfg(unix)]
    if let Err(error) = platform::process::restore_default_child_signal() {
        eprintln!("warning: cannot restore the default SIGCHLD disposition: {error}");
    }
    let exit_code = app::run();
    process::exit(exit_code);
}

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
    let exit_code = app::run();
    process::exit(exit_code);
}

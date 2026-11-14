// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Some(code) = natendo_app_lib::file_types_from_args() {
        std::process::exit(code);
    }
    natendo_app_lib::run()
}

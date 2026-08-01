#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    kestrel_local_lib::run();
}

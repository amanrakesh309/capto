// Release builds are tray-first and must not open a console window on Windows;
// this attribute is harmless on Linux and keeps the binary portable.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    capto_lib::run()
}

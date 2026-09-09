// 防止 Windows Release 弹出控制台窗口
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    miremote_desktop_lib::run()
}

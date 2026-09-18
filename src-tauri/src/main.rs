// Windows release 构建：隐藏控制台窗口
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    civilcalc_pc_lib::run()
}

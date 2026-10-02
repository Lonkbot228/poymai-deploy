// No console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("deploy") {
        #[cfg(windows)]
        unsafe {
            // Reuse the terminal we were started from so CLI output is visible.
            extern "system" {
                fn AttachConsole(pid: u32) -> i32;
            }
            AttachConsole(u32::MAX);
        }
        std::process::exit(poymai_deploy_lib::run_cli(&args));
    }
    poymai_deploy_lib::run_app();
}

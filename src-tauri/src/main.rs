#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Linux Wayland + NVIDIA WebKitGTK blank-screen fallback
    #[cfg(target_os = "linux")]
    {
        if std::env::args().any(|a| a == "--disable-gpu") {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
            std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
        }
    }

    openviking_desktop_lib::run();
}

//! On Windows, put the app icon and version details into develop.exe, so Explorer, the
//! taskbar, Task Manager and shortcuts show "Develop" with its icon.

fn main() {
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        if let Err(e) = embed_resource::compile("assets/app.rc", embed_resource::NONE).manifest_optional() {
            panic!("{e}");
        }
    }
}
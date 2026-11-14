//! Platform build steps.
//! - Windows: put the app icon and version details into develop.exe, so Explorer, the
//!   taskbar, Task Manager and shortcuts show "Develop" with its icon.
//! - macOS: link Clang's runtime library. ONNX Runtime's CoreML code uses `@available`
//!   checks, which call `___isPlatformVersionAtLeast` from that library; Rust links with
//!   `-nodefaultlibs`, so it is not included unless asked for.

fn main() {
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("windows") => {
            if let Err(e) = embed_resource::compile("assets/app.rc", embed_resource::NONE).manifest_optional() {
                panic!("{e}");
            }
        }
        Ok("macos") => link_clang_runtime(),
        _ => {}
    }
}

/// Find `libclang_rt.osx.a` in the toolchain's resource directory and link it.
fn link_clang_runtime() {
    let resource_dir = ["clang", "xcrun"].iter().find_map(|tool| {
        let mut command = std::process::Command::new(tool);
        if *tool == "xcrun" {
            command.arg("clang");
        }
        let output = command.arg("--print-resource-dir").output().ok()?;
        let dir = String::from_utf8(output.stdout).ok()?.trim().to_owned();
        (output.status.success() && !dir.is_empty()).then_some(dir)
    });
    match resource_dir {
        Some(dir) => {
            let lib_dir = std::path::Path::new(&dir).join("lib").join("darwin");
            if lib_dir.join("libclang_rt.osx.a").is_file() {
                println!("cargo:rustc-link-search=native={}", lib_dir.display());
                println!("cargo:rustc-link-lib=static=clang_rt.osx");
            } else {
                println!("cargo:warning=libclang_rt.osx.a not found in {}", lib_dir.display());
            }
        }
        None => println!("cargo:warning=could not ask clang for its resource directory"),
    }
}

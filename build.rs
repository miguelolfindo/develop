//! Platform build steps.
//! - Windows: put the app icon and version details into develop.exe, so Explorer, the
//!   taskbar, Task Manager and shortcuts show "Develop" with its icon.
//! - macOS: link Clang's runtime library. ONNX Runtime's CoreML code uses `@available`
//!   checks, which call `___isPlatformVersionAtLeast` from that library; Rust links with
//!   `-nodefaultlibs`, so it is not included unless asked for.

fn main() {
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("windows") => {
            // Fill the template with this version (so Explorer never shows an old number).
            let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
            let mut parts: Vec<String> = version.split(['.', '-']).take(3).map(str::to_owned).collect();
            parts.resize(3, "0".to_owned());
            let commas = format!("{},0", parts.join(","));
            let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
            let icon = std::path::Path::new(&dir).join("assets").join("icon.ico").display().to_string().replace('\\', "\\\\");
            let template = std::fs::read_to_string("assets/app.rc").expect("assets/app.rc");
            let rc = template.replace("@ICON@", &icon).replace("@COMMAS@", &commas).replace("@VERSION@", &version);
            let out = std::path::Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("app.rc");
            std::fs::write(&out, rc).expect("write app.rc");
            if let Err(e) = embed_resource::compile(&out, embed_resource::NONE).manifest_optional() {
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

use std::path::PathBuf;
use std::fs;

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo:rerun-if-env-changed=LLAMA_BIN_DIR");
    println!("cargo:rerun-if-env-changed=LLAMA_LIB_DIR");
    let bin_dir = std::env::var_os("LLAMA_BIN_DIR")
        .map(|path| PathBuf::from(&manifest_dir).join(path));
    let lib_dir = std::env::var_os("LLAMA_LIB_DIR")
        .map(|path| PathBuf::from(&manifest_dir).join(path));

    // Find target output directory
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let target_dir: PathBuf = out_dir
        .ancestors()
        .nth(3)
        .expect("can't find target dir")
        .to_path_buf();

    fs::create_dir_all(&target_dir).expect("failed to create output directory");
    println!("cargo:rerun-if-changed={}", manifest_dir);
    if let Some(bin_dir) = bin_dir {
        copy_llama_runtime(&bin_dir, lib_dir.as_ref(), &target_dir);
    } else {
        assert!(lib_dir.is_none(), "LLAMA_LIB_DIR requires LLAMA_BIN_DIR");
    }

    // sherpa-onnx-sys 2.0.1's build.rs calls copy_windows_runtime_dlls for
    // Windows shared builds, copying DLLs from its selected lib_dir into the
    // profile output directory (and examples). No cache scan is needed here.
}

fn copy_llama_runtime(bin_dir: &PathBuf, lib_dir: Option<&PathBuf>, target_dir: &PathBuf) {
    let exe_name = if cfg!(target_os = "windows") { "llama-server.exe" } else { "llama-server" };
    let src = bin_dir.join(exe_name);
    println!("cargo:rerun-if-changed={}", bin_dir.display());
    println!("cargo:rerun-if-changed={}", src.display());

    let dst = target_dir.join(exe_name);
    fs::copy(&src, &dst)
        .unwrap_or_else(|e| panic!("failed to copy {}: {e}", src.display()));

    // chmod +x
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&dst, fs::Permissions::from_mode(0o755));
    }

    if let Some(lib_dir) = lib_dir {
        println!("cargo:rerun-if-changed={}", lib_dir.display());
        for entry in fs::read_dir(lib_dir).expect("failed to read LLAMA_LIB_DIR") {
            let entry = entry.expect("failed to read runtime library entry");
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            let name_lower = name_str.to_lowercase();

            let is_lib = if cfg!(target_os = "windows") {
                name_lower.ends_with(".dll")
            } else if cfg!(target_os = "macos") {
                name_lower.ends_with(".dylib")
            } else {
                name_lower.ends_with(".so") || name_lower.contains(".so.")
            };

            if !is_lib || !entry.path().is_file() {
                continue;
            }

            let dst = target_dir.join(&*name);
            fs::copy(entry.path(), &dst)
                .unwrap_or_else(|e| panic!("failed to copy {}: {e}", entry.path().display()));
        }
    }

    println!("cargo:warning=copied llama-server{} to {}",
        if lib_dir.is_some() { " + runtime libs" } else { "" }, target_dir.display());
}

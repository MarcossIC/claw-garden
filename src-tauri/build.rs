fn main() {
    tauri_build::build();

    // Set minimum Windows version to 10.0 for the final binary and test binaries.
    //
    // Without this, Rust + GNU toolchain (MSYS2) defaults to subsystem version 5.2
    // (Windows XP x64). Windows then uses the legacy downlevel DLL resolution for
    // api-ms-win-*.dll. Those forwarder DLLs point to kernel32.WaitOnAddress which
    // doesn't exist — WaitOnAddress lives in kernelbase.dll on modern Windows.
    // With subsystem version 10.0, Windows uses the modern API set namespace and
    // resolves correctly, fixing STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139) on tests.
    //
    // Only needed for Windows targets; ignored on other platforms.
    #[cfg(target_os = "windows")]
    {
        // GNU (MSYS2 / MinGW) linker flags
        #[cfg(target_env = "gnu")]
        {
            println!("cargo:rustc-link-arg=-Wl,--major-os-version,10");
            println!("cargo:rustc-link-arg=-Wl,--minor-os-version,0");
            println!("cargo:rustc-link-arg=-Wl,--major-subsystem-version,10");
            println!("cargo:rustc-link-arg=-Wl,--minor-subsystem-version,0");
        }
        // MSVC linker flag. link.exe cannot set a subsystem version without
        // naming the subsystem, and this argument comes after the one
        // `windows_subsystem` emits, so it wins. It therefore has to name the
        // subsystem `main.rs` asks for, or a release build ships as a console
        // app that opens a terminal window behind the GUI.
        //
        // `main.rs` keys on `debug_assertions`, which Cargo does NOT expose to
        // build scripts (`CARGO_CFG_DEBUG_ASSERTIONS` is filtered out). `PROFILE`
        // is the closest reliable signal and matches it for the stock profiles:
        // `debug` (dev, test) has debug assertions, `release` (release, bench)
        // does not. The workspace defines no custom profiles.
        //
        // The windows subsystem makes the CRT look for `WinMain`; every target
        // here has a plain `main` (the test harness included), so the entry
        // point is named explicitly — the same pair `windows_subsystem` emits.
        #[cfg(target_env = "msvc")]
        {
            if std::env::var("PROFILE").as_deref() == Ok("release") {
                println!("cargo:rustc-link-arg=/SUBSYSTEM:windows,10.0");
                println!("cargo:rustc-link-arg=/ENTRY:mainCRTStartup");
            } else {
                println!("cargo:rustc-link-arg=/SUBSYSTEM:console,10.0");
            }
        }
    }
}

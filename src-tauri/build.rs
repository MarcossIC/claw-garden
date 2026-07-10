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
        // MSVC linker flag
        #[cfg(target_env = "msvc")]
        {
            println!("cargo:rustc-link-arg=/SUBSYSTEM:console,10.0");
        }
    }
}

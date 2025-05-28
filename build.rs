fn main() {
    // Set flags for Windows builds
    #[cfg(target_os = "windows")]
    {
        // Avoid parallel compilation to reduce file locking issues
        println!("cargo:rustc-cfg=build_parallel_disabled");
        // Use single codegen unit for Windows
        println!("cargo:rustc-flag=-Ccodegen-units=1");
    }
}

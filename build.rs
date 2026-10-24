fn main() {
    println!("cargo:rustc-check-cfg=cfg(foster_native_formatter)");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("HOST").ok() == std::env::var("TARGET").ok() {
        println!("cargo:rustc-cfg=foster_native_formatter");
    }
}

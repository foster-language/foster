use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rustc-check-cfg=cfg(foster_native_formatter)");
    println!("cargo:rerun-if-changed=../driver/format.fos");
    println!("cargo:rerun-if-changed=../../library");
    if env::var("HOST").ok() == env::var("TARGET").ok() {
        std::thread::Builder::new()
            .name("foster-formatter-build".into())
            .stack_size(16 * 1024 * 1024)
            .spawn(compile_formatter)
            .expect("cannot start formatter compiler")
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
    }
}

fn compile_formatter() {
    let source = fs::read_to_string("../driver/format.fos").expect("formatter source must exist");
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    // Compile the same formatting policy, with a consuming String entry boundary.
    let start = source
        .find("func main(arguments:")
        .expect("formatter entry");
    let end = source[start..]
        .find("\ntest ")
        .map(|end| start + end)
        .unwrap_or(source.len());
    let native_source = format!(
        "{}func main(arguments: Arguments) -> String [consume arguments] {{\n    format(arguments.values[0])\n}}\n{}",
        &source[..start],
        &source[end..]
    );
    let compilation =
        foster_compiler_core::compile(&native_source).expect("native formatter check");
    let object = foster_compiler_native::compile_object(&compilation, Default::default())
        .expect("native formatter compilation");
    let path = output.join("format.obj");
    fs::write(&path, &object.bytes).expect("native formatter object must be writable");
    fs::write(
        output.join("format_constants.rs"),
        format!(
            "pub const CONSTANTS: &[&str] = &{:?};",
            object.runtime_strings()
        ),
    )
    .expect("formatter constants must be writable");
    cc::Build::new().object(path).compile("foster_formatter");
    println!("cargo:rustc-cfg=foster_native_formatter");
}

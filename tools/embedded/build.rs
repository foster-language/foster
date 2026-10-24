use std::{env, fs, path::PathBuf};

fn main() {
    std::thread::Builder::new()
        .name("foster-tool-build".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(compile_tool)
        .expect("cannot start Foster tool compiler")
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}

fn compile_tool() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest
        .ancestors()
        .nth(3)
        .expect("embedded tool workspace root");
    let package = env::var("CARGO_PKG_NAME").unwrap();
    let name = package
        .strip_prefix("foster-tool-")
        .expect("embedded tool package name");
    let path = root.join(if name == "c-bridge" {
        "tools/cbind/src/bridge.fos".to_owned()
    } else {
        format!("tools/driver/{name}.fos")
    });
    println!("cargo:rerun-if-changed={}", path.display());
    println!("cargo:rerun-if-changed={}", root.join("library").display());
    let mut source = fs::read_to_string(&path).expect("embedded tool source must exist");
    if name == "c-bridge" {
        source = format!(
            "{source}\nfunc main(args: Arguments) -> List<String> {{\nbranch args.values[0] {{\n\"build\" -> [build(args.values[1], args.values[2], args.values[3], args.values[4])]\n_ -> generate_bridge(args.values[1], args.values[2])\n}}\n}}\n"
        );
    }
    let compilation = foster_compiler_core::compile(&source)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let program =
        foster_compiler_core::vm::compile(&compilation).expect("embedded bytecode compilation");
    let bytes =
        foster_compiler_core::vm::encode_program(&program).expect("embedded bytecode encoding");
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    fs::write(output.join("tool.fbc"), bytes).expect("embedded bytecode must be writable");
}

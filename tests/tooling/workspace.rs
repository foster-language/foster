use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

#[test]
fn compiler_facade_forwards_profiling_to_the_shared_core() {
    let output = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--format-version",
            "1",
            "--offline",
            "--features",
            "foster-compiler/compiler-profile",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let core = metadata["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|package| package["name"] == "foster-compiler-core")
        .unwrap();
    let core_node = metadata["resolve"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["id"] == core["id"])
        .unwrap();
    assert!(
        core_node["features"]
            .as_array()
            .unwrap()
            .iter()
            .any(|feature| feature == "compiler-profile")
    );
}

#[test]
fn compiler_core_and_bootstrap_do_not_depend_on_native_generation() {
    for (package, native) in [
        ("foster-bootstrap", false),
        ("foster-compiler-core", false),
        ("foster-compiler-native", true),
        ("foster-compiler", true),
        ("foster-native-formatter", true),
        ("foster-tool-init", false),
        ("foster-tool-project", false),
        ("foster-tool-format", false),
        ("foster-tool-documentation", false),
        ("foster-tool-c-bridge", false),
    ] {
        let output = Command::new(env!("CARGO"))
            .args([
                "tree",
                "--package",
                package,
                "--edges",
                "normal,build",
                "--prefix",
                "none",
                "--format",
                "{p}",
                "--offline",
            ])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let tree = String::from_utf8(output.stdout).unwrap();
        let has_cranelift = tree.lines().any(|line| {
            let name = line.split_whitespace().next().unwrap_or_default();
            name == "cranelift" || name.starts_with("cranelift-")
        });
        assert_eq!(has_cranelift, native, "{package} dependencies:\n{tree}");
        if !native {
            assert!(
                !tree.lines().any(|line| matches!(
                    line.split_whitespace().next(),
                    Some("foster-compiler" | "foster-compiler-native")
                )),
                "{package} must not depend on the compiler facade or native backend:\n{tree}"
            );
        }
    }
}

#[test]
fn execution_packages_build_without_compiler_dependencies() {
    let output = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--offline",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let packages = metadata["packages"].as_array().unwrap();
    let driver = packages
        .iter()
        .find(|package| package["name"] == "foster")
        .unwrap();
    assert!(
        driver["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .all(|dependency| dependency["kind"] != "build"),
        "driver builds must not regenerate tool artifacts through compiler build dependencies"
    );
    let names: BTreeSet<_> = packages
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        BTreeSet::from([
            "foster",
            "foster-bootstrap",
            "foster-compiler",
            "foster-compiler-core",
            "foster-compiler-native",
            "foster-vm",
            "foster-bytecode",
            "foster-native-runtime",
            "foster-host",
            "foster-native-formatter",
            "foster-tool-init",
            "foster-tool-project",
            "foster-tool-format",
            "foster-tool-documentation",
            "foster-tool-c-bridge",
        ])
    );
    let dependencies: BTreeMap<_, Vec<_>> = packages
        .iter()
        .map(|package| {
            (
                package["name"].as_str().unwrap(),
                package["dependencies"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|d| d["kind"] != "dev")
                    .map(|d| d["name"].as_str().unwrap())
                    .collect(),
            )
        })
        .collect();
    for root in [
        "foster-vm",
        "foster-bytecode",
        "foster-native-runtime",
        "foster-host",
    ] {
        let mut pending = vec![root];
        let mut visited = BTreeSet::new();
        while let Some(package) = pending.pop() {
            if !visited.insert(package) {
                continue;
            }
            assert!(
                !matches!(
                    package,
                    "foster"
                        | "foster-compiler"
                        | "foster-compiler-core"
                        | "foster-compiler-native"
                        | "foster-bootstrap"
                ),
                "{root} depends on {package}"
            );
            if let Some(children) = dependencies.get(package) {
                pending.extend(children.iter().copied());
            }
        }
    }
}

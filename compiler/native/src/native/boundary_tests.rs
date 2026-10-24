use super::*;
use crate::compiler::profile::collect_counters as collect;

#[test]
fn native_and_vm_reuse_the_same_immutable_shared_programs() {
    let compilation = crate::compile("func main() -> Int { 20 + 22 }").unwrap();
    let baseline = compilation.shared_program(false).unwrap();
    let optimized = compilation.shared_program(true).unwrap();
    let before = format!("{:?}", baseline.functions());
    for optimize in [true, false, true] {
        crate::vm::compile_with_options(&compilation, crate::vm::CompileOptions { optimize })
            .unwrap();
        let _prepared = prepare_with_options(&compilation, CompileOptions { optimize }).unwrap();
        assert!(std::sync::Arc::ptr_eq(
            &baseline,
            &compilation.shared_program(false).unwrap()
        ));
        assert!(std::sync::Arc::ptr_eq(
            &optimized,
            &compilation.shared_program(true).unwrap()
        ));
    }
    assert_eq!(before, format!("{:?}", baseline.functions()));
}

#[test]
fn shared_scalar_pass_folds_native_ir_without_moving_sites() {
    let compilation = crate::compile("func main() -> Int { (20 + 22) * 2 }").unwrap();
    let prepared = prepare(&compilation).unwrap();
    let mut function = prepared.functions()[0].ir().clone();
    let sites = |function: &ir::Function| {
        function
            .blocks
            .iter()
            .map(|block| {
                block
                    .instructions
                    .iter()
                    .map(|entry| entry.span.clone())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    };
    let before = sites(&function);
    assert!(
        crate::codegen::backend::fold_scalars(
            &mut function,
            &HashMap::new(),
            &prepared.program.metadata.constants,
        )
        .unwrap()
            > 0
    );
    assert_eq!(sites(&function), before);
}

#[test]
fn native_reference_parameter_contracts_preserve_storage_and_dispatch() {
    const SOURCE: &str =
        include_str!("../../../../tests/fixtures/programs/contract_reference_parameters.fos");
    let check = |source: &str| {
        let compilation = crate::compile(source).unwrap();
        for optimize in [false, true] {
            compile_object(&compilation, CompileOptions { optimize }).unwrap();
        }
    };
    for source in [
        SOURCE,
        include_str!("../../../../tests/fixtures/programs/implicit_parameter_contracts.fos"),
        include_str!("../../../../tests/fixtures/programs/cursor_dispatch.fos"),
        include_str!("../../../../tests/fixtures/programs/structural_reference_fields.fos"),
    ] {
        check(source);
    }
    for (declaration, annotation) in [
        ("type Alias = Replace", "Alias"),
        ("type More = & Replace & {}", "More"),
        ("type Marker = { pub value: Int }", "Replace & Marker"),
    ] {
        check(&format!("{SOURCE}\n{declaration}").replace(": Replace", &format!(": {annotation}")));
    }
    let required = SOURCE.replace("type Replace =", "pub type Replace =").replace("func main() -> Int", r#"
pub type Updater = { pub func apply(item: Replace) -> () [reshape item] }
pub type Implementation = & Updater & {}
impl Implementation {
    pub func apply(item: Replace) -> () [reshape item] { update(item) }
}
func invoke(updater: Updater, item: Replace) -> () [reshape item] {
    updater.apply(item)
}
func main() -> Int"#).replace("pair(holder.first, holder.second)", "let updater = Implementation {}\ninvoke(updater, holder.first)\ninvoke(updater, holder.second)");
    check(&required);
}

#[test]
fn native_preparation_analyzes_each_body_once_across_specializations() {
    let compilation = crate::compile("func identity<T>(value: T) -> T { value }\nfunc main() -> Int { assert(identity(true))\nassert(identity(1.5) == 1.5)\nidentity(42) }").unwrap();
    let (prepared, report) = collect(|| crate::native::prepare(&compilation));
    let prepared = prepared.unwrap();
    let bodies = prepared
        .functions()
        .iter()
        .map(|function| function.source_function())
        .collect::<std::collections::HashSet<_>>();
    assert!(prepared.functions().len() > bodies.len());
    assert!(!report.contains_key("native.flow_analysis"));
    let shared = crate::vm::compile_shared(&compilation).unwrap();
    assert_eq!(report["shared.flow_analysis"], shared.functions().len());
}

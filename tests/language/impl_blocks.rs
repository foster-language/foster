use foster::vm::Value;

#[test]
fn implicit_receivers_are_present_even_when_unused_and_static_members_have_none() {
    let source = r#"
type Counter = { value: Int }
impl Counter {
    static func new(value: Int) -> Self { Counter { value } }
    func answer() -> Int { 42 }
    func bump() -> () [mut self] { self.value = self.value + 1 }
    func borrow() -> ref[self] Self [read self] { ref self }
}
func main() -> Int {
    let counter = Counter.new(40)
    counter.bump()
    assert(counter.borrow().value == 41)
    counter.answer()
}
"#;
    assert_eq!(foster::run(source).unwrap(), Value::Integer(42));
    let parsed = foster::parse(source).unwrap();
    let factory = &parsed.functions[0];
    assert!(!factory.receiver);
    assert_eq!(factory.parameters.len(), 1);
    let method = &parsed.functions[1];
    assert!(method.receiver);
    assert_eq!(method.parameters[0].name, "self");
    assert!(method.parameters[0].span.is_empty());
    assert!(foster::compile(&source.replace("counter.answer()", "Counter.answer()")).is_err());
}

#[test]
fn explicit_receivers_and_module_static_functions_are_rejected() {
    for source in [
        "type Item = {}\nimpl Item { func get(self: Self) {} }",
        "type Item = {}\nimpl Item { func get(self: ref[self] Self) {} }",
        "type Item = { func get(self: Self) -> Int }",
        "static func main() {}",
        "pub static func main() {}",
        "ref func main() {}",
        "type Item = {}\nimpl Item { ref func get() {} }",
        "type Item = { ref func get() -> Int }",
        "type Item = {}\nimpl Item { ref static func get() {} }",
        "type Item = {}\nimpl Item { static func get() -> Int { self.value } }",
    ] {
        assert!(foster::compile(source).is_err(), "accepted {source}");
    }
}

#[test]
fn implicit_receivers_replace_the_callers_binding() {
    let source = r#"
type Item = { value: Int }
impl Item {
    func replace(value: Int) -> () [reshape self] { self = Item { value } }
}
func main() -> Int {
    let item = Item { value: 1 }
    item.replace(42)
    item.value
}
"#;
    assert_eq!(foster::run(source).unwrap(), Value::Integer(42));
}

#[test]
fn specialized_impl_targets_supply_nested_and_concrete_receiver_types() {
    let source = r#"
type Box<T> = { value: T }
impl<T> Box<Box<T>> {
    func unwrap() -> T { self.value.value }
}
impl<> Box<Int> {
    static func new(value: Int) -> Self { Box { value } }
    func increment() -> Int { self.value + 1 }
}
func main() -> Int {
    Box { value: Box { value: 20 } }.unwrap() + Box.new(21).increment()
}
"#;
    assert_eq!(foster::run(source).unwrap(), Value::Integer(42));
}

#[test]
fn generic_blocks_supply_receiver_types_and_preserve_member_generics() {
    let source = r#"
type Box<T> = { value: T }
impl Box<T> {
    static func new(value: T) -> Box<T> { Box { value } }
    func get() -> T { self.value }
    func apply<U>(transform: func(T) -> U) -> U { transform(self.value) }
}
impl Box<T> {
    func again() -> T { self.get() }
}
func main() -> Int {
    let box = Box.new(20)
    box.again() + box.apply((value: Int) -> value + 2)
}
"#;
    assert_eq!(foster::run(source).unwrap(), Value::Integer(42));
    let program = foster::parse(source).unwrap();
    assert_eq!(program.implementations.len(), 2);
    let apply = program
        .functions
        .iter()
        .find(|function| function.name == "Box.apply")
        .unwrap();
    assert_eq!(apply.type_parameters, ["T", "U"]);
    assert!(apply.receiver);
    assert!(source[apply.span.clone()].starts_with("func apply"));
}

#[test]
fn enum_blocks_support_implicit_self_receivers() {
    let source = r#"
enum Choice = Value(Int) | Empty
impl Choice {
    func get() -> Int {
        branch self {
            Choice.Value(value) -> value
            Choice.Empty -> 0
        }
    }
}
func main() -> Int { Choice.Value(42).get() }
"#;
    assert_eq!(foster::run(source).unwrap(), Value::Integer(42));
}

#[test]
fn rejects_invalid_block_structure_and_old_member_declarations() {
    for source in [
        "type Box = {}\nfunc Box.new() { Box {} }",
        "type Box = {}\nfunc new(self: Box) {}",
        "type Box = {}\nimpl Box { func Box.new() {} }",
        "type Box = {}\nimpl Box { impl Box {} }",
        "type Box = {}\nimpl Box { type Inner = {} }",
        "type Box = {}\nimpl Box { static func new() {}",
        "type Box<T> = {}\nimpl Box<T, T> {}",
        "type Box<T> = {}\nimpl Box<T> { func get<T>() {} }",
        "pub impl Box {}",
        "func main() { impl Box {} }",
    ] {
        assert!(foster::parse(source).is_err(), "accepted {source}");
    }
    for source in [
        "impl Missing {}\nfunc main() {}",
        "type Box<T> = {}\nimpl Box<T, U> {}\nfunc main() {}",
        "type Box = {}\nimpl Box { func get() {} }\nimpl Box { func get() {} }\nfunc main() {}",
    ] {
        assert!(foster::compile(source).is_err(), "accepted {source}");
    }
}

#[test]
fn recovery_does_not_leak_members_from_a_damaged_block() {
    let source = "type Box = {}\nimpl Box {\n    func broken() { let x = }\n    func later() {}\n}\nfunc healthy() { 42 }";
    let parsed = foster::parse_recovering(source).unwrap();
    assert_eq!(parsed.diagnostics.len(), 1);
    assert_eq!(parsed.program.functions.len(), 1);
    assert_eq!(parsed.program.functions[0].name, "healthy");
    assert!(parsed.program.implementations.is_empty());
}

#[test]
fn formatter_keeps_member_documentation_inside_the_block() {
    let source =
        "type Box = {}\nimpl Box {\n/// Creates a box.\npub static func new() {\nBox {}\n}\n}\n";
    let formatted = foster::formatter::format(source).unwrap();
    assert!(formatted.contains(
        "impl Box {\n    /// Creates a box.\n    pub static func new() {\n        Box {}\n    }\n}"
    ));
    assert_eq!(formatted, foster::formatter::format(&formatted).unwrap());
    let parsed = foster::parse(&formatted).unwrap();
    assert_eq!(
        parsed.functions[0].documentation.as_deref(),
        Some("Creates a box.")
    );
}

#[test]
fn implicit_receiver_replacement_can_infer_effects_and_reach_projected_storage() {
    let source = include_str!("../fixtures/programs/implicit_receiver_replacement.fos");
    assert_eq!(foster::run(source).unwrap(), Value::Integer(42));
}

#[test]
fn receiver_replacement_invalidates_live_borrows() {
    let source = r#"
type Item = { value: Int }
impl Item {
    func borrow() -> ref[self] Item { ref self }
    func replace() -> () [reshape self] { self = Item { value: 42 } }
}
func main() -> Int {
    let item = Item { value: 1 }
    let borrowed = item.borrow()
    item.replace()
    borrowed.value
}
"#;
    assert!(foster::compile(&source.replace("    item.replace()\n", "")).is_ok());
    assert!(foster::compile(source).is_err());
}

#[test]
fn receiver_replacement_requires_a_reshaping_contract() {
    let source = "type Item = { value: Int }\nimpl Item { func replace() -> () [mut self] { self = Item { value: 42 } } }\nfunc main() -> Int { 42 }";
    let error = foster::compile(source)
        .err()
        .expect("replacement must require reshaping");
    assert!(error.message.contains("reshape"), "{error}");
}

#[test]
fn receiver_replacement_invalidates_borrows_inside_the_method() {
    let source = r#"
type Item = { value: Int }
impl Item {
    func replace() -> Int [reshape self] {
        let borrowed = ref self
        self = Item { value: 42 }
        borrowed.value
    }
}
func main() -> Int { Item { value: 1 }.replace() }
"#;
    assert!(foster::compile(source).is_err());
}

#[test]
fn captured_receiver_replacement_requires_a_reshaping_contract() {
    let source = r#"
type Item = { value: Int }
impl Item {
    func replace() -> () [mut self] {
        let change = [ref self] () -> { self = Item { value: 42 }
() }
        change()
    }
}
func main() -> Int { 42 }
"#;
    let error = foster::compile(source)
        .err()
        .expect("replacement must require reshaping");
    assert!(error.message.contains("reshape"), "{error}");
}

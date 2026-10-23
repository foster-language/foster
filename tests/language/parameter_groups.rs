use foster::vm::Value;

const SOURCE: &str = include_str!("../fixtures/programs/parameter_groups.fos");
const NESTED_REFERENCE_CALLS: &str =
    include_str!("../fixtures/programs/nested_reference_calls.fos");

#[test]
fn implicit_parameter_storage_allows_purer_method_implementations() {
    assert_eq!(
        foster::run(include_str!(
            "../fixtures/programs/implicit_parameter_contracts.fos"
        ))
        .unwrap(),
        Value::Integer(42)
    );
}

#[test]
fn redundant_incoming_reference_annotations_are_rejected() {
    for source in [
        "func borrow(foo: ref[foo] Int) -> ref[foo] Int { ref foo }",
        "type Contract = { pub func borrow(foo: ref[foo] Int) -> ref[foo] Int }",
        "type Item = {}\nimpl Item { func use(foo: ref[foo] Int) -> () { () } }",
        "type Item = {}\nimpl Item { static func use(foo: ref[foo] Int) -> () { () } }",
        "func main() -> Int { let callback = (foo: ref[foo] Int) -> foo\n42 }",
    ] {
        let error = foster::parse(source).unwrap_err();
        assert!(error.message.contains("redundant `ref[foo]`"), "{error}");
        assert!(foster::formatter::format(source).is_err());
    }
}

#[test]
fn explicit_reference_relationships_remain_available() {
    let source = r#"
func select(owner: Int, view: ref[owner] Int) -> ref[owner] Int { ref view }
func main() -> Int {
    let original = 42
    let selected = select(original, ref original)
    selected.copy()
}
"#;
    assert_eq!(foster::run(source).unwrap(), Value::Integer(42));
    let formatted = foster::formatter::format(source).unwrap();
    assert!(formatted.contains("view: ref[owner] Int"));
    let nested = "func use(items: List<ref[items] Int>, callback: func(ref[items] Int) -> ref[items] Int) -> () { () }";
    let formatted = foster::formatter::format(nested).unwrap();
    assert!(formatted.contains("items: List<ref[items] Int>"));
    assert!(formatted.contains("func(ref[items] Int) -> ref[items] Int"));
}

#[test]
fn borrowed_contract_arguments_preserve_generic_reference_provenance() {
    let source = r#"
type Value = { pub value: Int }
func borrowed(item: Value) -> ref[item] Value { ref item }
func inspect(item: Value) -> Int { item.value }
func identity<T & Value>(item: T) -> T { item }
func main() -> Int {
    let item = Value { value: 42 }
    let result = identity(borrowed(item))
    inspect(result)
}
"#;
    assert_eq!(foster::run(source).unwrap(), Value::Integer(42));
    let escaped = source.replace(
        "let item = Value { value: 42 }\n    let result = identity(borrowed(item))",
        "let result = :work { let item = Value { value: 42 }\nidentity(borrowed(item)) }",
    );
    let error = foster::compile(&escaped).unwrap_err();
    assert_eq!(error.code.as_deref(), Some("E0401"), "{error}");
}

#[test]
fn reference_constraint_checks_preserve_self_result_obligations() {
    let error = foster::compile(
        r#"
pub type Copyable = { pub value: Int, pub func duplicate() -> Self }
impl Copyable {
    pub func duplicate() -> Self { Copyable { value: self.value } }
}
func borrowed(item: Copyable) -> ref[item] Copyable { ref item }
func identity<T & Copyable>(item: T) -> T { item }
func main() -> Int {
    let item = Copyable { value: 42 }
    identity(borrowed(item)).value
}
"#,
    )
    .unwrap_err();
    assert!(error.message.contains("duplicate"), "{error}");
}

#[test]
fn nested_reference_calls_preserve_original_storage() {
    assert_eq!(
        foster::run(NESTED_REFERENCE_CALLS).unwrap(),
        Value::Integer(42)
    );
}

#[test]
fn nested_reference_calls_reject_expired_or_invalidated_owners() {
    let declarations = NESTED_REFERENCE_CALLS.split("func main()").next().unwrap();
    for body in [
        "let result = borrow(borrow(42))\nresult.copy()",
        "let callback = borrow\nlet result = callback(callback(42))\nresult.copy()",
        "let result = Item { value: 42 }.borrowed().borrowed()\nresult.read()",
        "let original = 42\nlet result = borrow(borrow(original))\noriginal = 43\nresult.copy()",
        "let callback = borrow\nlet original = 42\nlet result = callback(callback(original))\noriginal = 43\nresult.copy()",
        "let result = :work { let original = 42\nborrow(borrow(original)) }\nresult.copy()",
        "let item = Item { value: 42 }\nlet result = borrowed_item(item).borrowed()\nitem.replace(43)\nresult.read()",
        "let item = Item { value: 42 }\nlet result = borrowed_item(item).borrowed()\nlet moved = move item\nresult.read()",
    ] {
        let source = format!("{declarations}\nfunc main() -> Int {{\n{body}\n}}");
        let error = foster::compile(&source).unwrap_err();
        assert_eq!(error.code.as_deref(), Some("E0401"), "{body}: {error}");
    }
}

#[test]
fn parameter_names_expose_storage_without_group_declarations() {
    assert_eq!(foster::run(SOURCE).unwrap(), Value::Integer(42));
}

#[test]
fn parameter_groups_reject_wrong_origins_and_expired_storage() {
    for source in [
        "func borrow(value: Int) -> ref[value] Int { ref value }\nfunc main() { let values = [1]\nlet selected = borrow(values[0])\nvalues.push(2)\nselected.copy() }",
        "type Item = { value: Int }\nfunc borrow(item: Item) -> ref[item] Item { ref item }\nfunc main() { let callback = borrow\nlet escaped = :work { let local = Item { value: 1 }\ncallback(local) }\nescaped.value }",
        "type Item = { value: Int }\nfunc bad(left: Item, right: Item) -> ref[left] Item { ref right }",
        "type Item = { value: Int }\nfunc bad(value: Item) -> ref[missing] Item { ref value }",
        "type Item = { value: Int }\nfunc bad(value: Item) -> ref[value] Item [consume value] { ref value }",
        "type Item = { value: Int }\nfunc bad() -> ref[work] Item { :work { let item = Item { value: 1 }\nref item } }",
        "type Item = { value: Int }\nfunc borrow(item: Item) -> ref[item] Item { ref item }\nfunc main() { let escaped = :work { let local = Item { value: 1 }\nborrow(local) }\nescaped.value }",
    ] {
        assert!(foster::compile(source).is_err(), "accepted: {source}");
    }
}

#[test]
fn separate_group_declarations_are_rejected() {
    assert!(
        foster::parse("func borrow[g: group Int](value: ref[g] Int) -> ref[g] Int { ref value }")
            .is_err()
    );
}

#[test]
fn ordinary_parameter_origins_survive_callable_storage() {
    foster::compile(
        r#"
type Item = { value: Int }
func first(item: Item) -> ref[item] Item { ref item }
func second(other: Item) -> ref[other] Item { ref other }
func main() -> Int {
    let item = Item { value: 42 }
    let callbacks = [first, second]
    let borrowed = callbacks[0](item)
    borrowed.value
}
"#,
    )
    .unwrap();
}

#[test]
fn returned_closure_effects_follow_renamed_parameters() {
    foster::compile(
        r#"
type Item = { value: Int }
func reader(item: Item) -> func() -> Int [read item] { [ref item] () -> item.value }
func relay(other: Item) -> func() -> Int [read other] { reader(other) }
func main() -> Int { let item = Item { value: 42 }
let callback = relay(item)
callback() }
"#,
    )
    .unwrap();
}

#[test]
fn method_contracts_use_parameter_positions_for_group_names() {
    foster::compile(
        r#"
type Item = { value: Int }
type Reader = { pub func borrow(item: Item) -> ref[item] Item }
type Implementation = & Reader & {}
impl Implementation {
    func borrow(other: Item) -> ref[other] Item { ref other }
}
func main() -> Int {
    let reader = Implementation {}
    let item = Item { value: 42 }
    reader.borrow(item).value
}
"#,
    )
    .unwrap();
}

#[test]
fn call_substitution_does_not_rebind_an_actual_parameter_name() {
    foster::compile(
        r#"
type Item = { value: Int }
func first(left: Item, right: Item) -> ref[left] Item { ref left }
func relay(right: Item, other: Item) -> ref[right] Item { first(right, other) }
impl Item {
    func choose(item: Item) -> ref[item] Item { ref item }
    func forward(other: Item) -> ref[self] Item { other.choose(self) }
}
func main() -> Int { let item = Item { value: 42 }
relay(item, item).value }
"#,
    )
    .unwrap();
}

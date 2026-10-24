// Shared by Criterion and compiler regressions so scaling inputs stay executable.
pub fn candidates(count: usize) -> String {
    let mut source = String::from(
        "pub type Answer = { pub func answer() -> Int [read self] }\n\
         pub type Envelope<T> = { pub value: T }\n\
         pub type Provider = & Answer & { pub func build() -> Envelope<Self> [read self] }\n\
         func build(provider: Provider) -> Envelope<Provider> { provider.build() }\n",
    );
    for index in 0..count {
        source.push_str(&format!(
            "pub type Item{index} = & Provider & {{ pub number: Int }}\n\
             impl Item{index} {{\n\
               pub func answer() -> Int {{ self.number }}\n\
               pub func build() -> Envelope<Item{index}> {{ Envelope {{ value: Item{index} {{ number: self.number }} }} }}\n\
             }}\n\
             pub type Other{index} = {{ pub number: Int }}\n\
             impl Other{index} {{ pub func unrelated{index}() -> Int {{ self.number }} }}\n"
        ));
    }
    source.push_str("func main() -> Int {\nlet total = 0\n");
    for index in 0..count {
        source.push_str(&format!(
            "let item{index} = build(Item{index} {{ number: 42 }})\n\
             total = total + item{index}.value.answer()\n\
             let other{index} = Other{index} {{ number: 0 }}\n\
             total = total + other{index}.unrelated{index}()\n"
        ));
    }
    source.push_str(&format!("assert(total == {})\n42\n}}\n", count * 42));
    source
}

pub fn views(depth: usize) -> String {
    let mut source = String::new();
    for level in 0..=depth {
        source.push_str(&format!("type View{level} = {{ pub number: Int"));
        for field in 0..level {
            source.push_str(&format!(", pub extra{field}: Int"));
        }
        source.push_str(" }\n");
    }
    for level in 0..depth {
        source.push_str(&format!(
            "func step{level}() -> View{level} {{ step{}() }}\n",
            level + 1
        ));
    }
    source.push_str(&format!(
        "func step{depth}() -> View{depth} {{ View{depth} {{ number: 42"
    ));
    for field in 0..depth {
        source.push_str(&format!(", extra{field}: {field}"));
    }
    source.push_str(" } }\nfunc main() -> Int { step0().number }\n");
    source
}

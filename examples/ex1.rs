use std::any::TypeId;
use std::collections::HashSet;

struct S1;

type Signals = HashSet<TypeId>;

struct A;

trait Node {
    fn process(&mut self, _signals: &mut Signals);
}

impl Node for A {
    fn process(&mut self, _signals: &mut Signals) {
        _signals.insert(TypeId::of::<S1>());
    } // Added missing closing brace
}

fn main() {
    println!("Hello, world!");

    let mut tree = A;

    tree.process(&mut std::collections::HashSet::new());
}

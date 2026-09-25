// Naming a concrete platform tree or the selected `platform_imp` alias is
// denied outside the owner's crate-root bridge, even in private code.
fn uses_imp_tree() {
    let platform_imp = ();
    let _ = platform_imp;
}

fn main() {}

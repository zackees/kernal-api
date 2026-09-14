#[path = "../../shared/compiler_policy.rs"]
mod compiler_policy;

mod bindings {
    use super::Sketch;
    wit_bindgen::generate!({ path: "../../../src/guest_component_compiler.wit", world: "compiler-proof" });
    export!(Sketch);
}

struct Sketch;
impl bindings::Guest for Sketch {
    async fn run() -> Result<(), ()> {
        compiler_policy::proof().await.map_err(|_| ())
    }
}

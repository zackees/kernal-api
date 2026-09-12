#![cfg(feature = "wasmtime45-integration")]

#[path = "fixtures/wasmtime45_scalar_host.rs"]
mod generated;

struct Host;

impl generated::KernalApiV1Imports for Host {
    fn checked(&mut self, flag: bool, tiny: u8, total: u64) -> wasmtime::Result<bool> {
        Ok(flag && tiny == 7 && total == u64::MAX)
    }

    fn reset(&mut self) -> wasmtime::Result<()> {
        Ok(())
    }
}

#[test]
fn generated_wasmtime45_linker_glue_compiles_and_registers() {
    let engine = wasmtime::Engine::default();
    let mut linker = wasmtime::Linker::<Host>::new(&engine);
    generated::link_kernal_api_v1(&mut linker).unwrap();
}

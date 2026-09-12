//! Linked Rust artifact proving the generated contract before operations exist.

#[allow(dead_code)]
#[rustfmt::skip]
#[path = "../../../abi/generated/src/lib.rs"]
mod generated;

mod contract_v1 {
    use super::generated::imports;

    #[derive(Debug)]
    pub(super) struct IncompatibleHost;

    pub(super) fn verify_reserved_contract() -> Result<(), IncompatibleHost> {
        let invalid = |_| IncompatibleHost;
        if imports::abi_version().map_err(invalid)? != 1
            || imports::capability_bits().map_err(invalid)? != 0
            || imports::submit(0, 0, 0, 0).map_err(invalid)? != 0
            || imports::poll(0).map_err(invalid)? != 3
            || imports::completion_word(0, 0).map_err(invalid)? != 2
            || imports::completion_word(0, 1).map_err(invalid)? != u64::MAX
            || imports::cancel(0).map_err(invalid)? != 2
            || imports::release(0).map_err(invalid)? != 2
            || imports::yield_now().map_err(invalid)? != 7
        {
            return Err(IncompatibleHost);
        }
        Ok(())
    }
}

#[export_name = "kernal-api-run"]
pub extern "C" fn kernal_api_run() -> u32 {
    contract_v1::verify_reserved_contract().expect("root generated contract");
    // std owns the thread bootstrap. Every kernel operation in each Store
    // crosses the same generated imports; the fixture declares no extern ABI.
    let child = std::thread::spawn(|| {
        contract_v1::verify_reserved_contract().expect("child generated contract");
    });
    child.join().expect("generated guest thread joined");
    0
}

fn main() {
    let _ = kernal_api_run();
}

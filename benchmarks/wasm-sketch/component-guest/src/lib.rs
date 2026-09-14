//! Private Component candidate fixtures, selected at compile time.
std::cfg_select! {
    feature = "lowering-trap-proof" => { mod compiler_lowering_client; }
    feature = "compiler-proof" => { mod compiler_client; }
    _ => { mod legacy; }
}

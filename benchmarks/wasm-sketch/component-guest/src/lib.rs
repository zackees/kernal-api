//! Private Component candidate fixtures, selected at compile time.
std::cfg_select! {
    feature = "compiler-proof" => { mod compiler_client; }
    _ => { mod legacy; }
}

// aux-build:framehop.rs
// aux-build:tokio.rs

// Implementing a backend's own trait for an exported facade type puts the
// backend in the caller's import list, because the trait has to be in scope to
// call the method. The async mirrors of `std::io` are the stated exception.
// Compiled under the facade owner's crate name so the lint applies the owner
// rule rather than the client rule.
#![allow(unused)]
#![crate_name = "kernal_api"]

extern crate framehop;
extern crate tokio;

pub struct FacadeStream(u8);

// The shape #109 names: a backend extension point on an exported type.
impl framehop::ModuleSectionInfo<Vec<u8>> for FacadeStream {}

// Per-trait, not per-crate: `tokio` is exempt only for the io vocabulary.
impl tokio::net::ToSocketAddrs for FacadeStream {}

// The foundational async io vocabulary, which a usable stream facade must
// speak for any combinator or codec to accept it.
impl tokio::io::AsyncRead for FacadeStream {}

impl tokio::io::AsyncWrite for FacadeStream {}

// A private self type publishes nothing: no client can name the type, so the
// backend trait never reaches an import list.
struct PrivateAdapter(u8);

impl framehop::ModuleSectionInfo<Vec<u8>> for PrivateAdapter {}

// The reverse direction stays legal. Only a caller that already holds the
// backend type can reach this impl, so it imposes no vocabulary on one that
// does not.
pub trait FacadeConversion {
    fn convert(&self);
}

impl FacadeConversion for tokio::sync::Mutex<u8> {
    fn convert(&self) {}
}

fn main() {}

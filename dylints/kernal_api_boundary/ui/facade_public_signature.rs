// aux-build:crash_handler.rs
// aux-build:tokio.rs

// The facade owner may use a backend privately; it may not name one where a
// client has to. Compiled under the facade owner's crate name so the lint
// applies the owner rule rather than the client rule.
#![allow(unused)]
#![crate_name = "kernal_api"]

extern crate crash_handler;
extern crate tokio;

use crash_handler::Error;
use tokio::sync::Mutex;

/// The shape that reached the released facade before #84: a backend type in
/// the payload of a public enum variant.
pub enum InstallError {
    Handler(Error),
}

pub struct PublicField {
    pub lock: Mutex<u8>,
}

pub fn public_parameter(lock: Mutex<u8>) {}

pub fn public_return() -> Option<Error> {
    None
}

pub type PublicAlias = Mutex<u8>;

pub fn public_bound<T: tokio::io::AsyncRead>(value: T) {}

pub trait PublicSupertrait: tokio::io::AsyncRead {}

pub struct PublicNewtype(u8);

impl PublicNewtype {
    pub fn inherent_return(&self) -> Option<Error> {
        None
    }
}

// A private tuple field of a public newtype is the established facade shape
// and stays legal, as do private items, internal use, and the trait
// implementations that adapt a facade type into a backend one.
pub struct PrivateField(Mutex<u8>);

impl From<PublicNewtype> for Mutex<u8> {
    fn from(value: PublicNewtype) -> Self {
        Mutex::new(value.0)
    }
}

struct PrivateItem {
    pub lock: Mutex<u8>,
}

fn internal(lock: Mutex<u8>) -> Option<Error> {
    None
}

fn main() {}

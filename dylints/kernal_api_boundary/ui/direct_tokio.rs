// aux-build:tokio.rs

#![allow(unused)]

extern crate tokio;

use tokio::sync::Mutex;

fn bypasses_boundary() {
    let lock = Mutex::new(1_u8);
    tokio::task::yield_now();
    drop(lock);
}

fn main() {}

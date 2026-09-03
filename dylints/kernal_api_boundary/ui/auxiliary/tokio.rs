pub mod sync {
    pub struct Mutex<T>(T);

    impl<T> Mutex<T> {
        pub fn new(value: T) -> Self {
            Self(value)
        }
    }
}

pub mod task {
    pub fn yield_now() {}
}

pub mod io {
    pub trait AsyncRead {}

    pub trait AsyncWrite {}
}

pub mod net {
    pub trait ToSocketAddrs {}
}

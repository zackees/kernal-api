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

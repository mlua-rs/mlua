#[allow(unused_macros)]
macro_rules! cstr {
    ($s:expr) => {
        concat!($s, "\0") as *const str as *const [::std::ffi::c_char] as *const ::std::ffi::c_char
    };
}

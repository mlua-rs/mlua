cfg_if::cfg_if! {
    if #[cfg(all(feature = "lua54", not(any(feature = "lua53", feature = "lua52", feature = "lua51", feature = "luajit", feature = "luau"))))] {
        include!("main_inner.rs");
    } else if #[cfg(all(feature = "lua53", not(any(feature = "lua54", feature = "lua52", feature = "lua51", feature = "luajit", feature = "luau"))))] {
        include!("main_inner.rs");
    } else if #[cfg(all(feature = "lua52", not(any(feature = "lua54", feature = "lua53", feature = "lua51", feature = "luajit", feature = "luau"))))] {
        include!("main_inner.rs");
    } else if #[cfg(all(feature = "lua51", not(any(feature = "lua54", feature = "lua53", feature = "lua52", feature = "luajit", feature = "luau"))))] {
        include!("main_inner.rs");
    } else if #[cfg(all(feature = "luajit", not(any(feature = "lua54", feature = "lua53", feature = "lua52", feature = "lua51", feature = "luau"))))] {
        include!("main_inner.rs");
    } else if #[cfg(all(feature = "luau", not(any(feature = "lua54", feature = "lua53", feature = "lua52", feature = "lua51", feature = "luajit"))))] {
        include!("main_inner.rs");
    } else {
        fn main() {
            compile_error!("You can enable only one of the features: lua54, lua53, lua52, lua51, luajit, luajit52, luau");
        }
    }
}

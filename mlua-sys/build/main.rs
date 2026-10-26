cfg_if::cfg_if! {
    if #[cfg(all(feature = "lua55", not(any(feature = "lua54", feature = "lua53", feature = "lua52", feature = "lua51", feature = "luajit", feature = "luau"))))] {
        include!("main_inner.rs");
    } else if #[cfg(all(feature = "lua54", not(any(feature = "lua55", feature = "lua53", feature = "lua52", feature = "lua51", feature = "luajit", feature = "luau"))))] {
        include!("main_inner.rs");
    } else if #[cfg(all(feature = "lua53", not(any(feature = "lua55", feature = "lua54", feature = "lua52", feature = "lua51", feature = "luajit", feature = "luau"))))] {
        include!("main_inner.rs");
    } else if #[cfg(all(feature = "lua52", not(any(feature = "lua55", feature = "lua54", feature = "lua53", feature = "lua51", feature = "luajit", feature = "luau"))))] {
        include!("main_inner.rs");
    } else if #[cfg(all(feature = "lua51", not(any(feature = "lua55", feature = "lua54", feature = "lua53", feature = "lua52", feature = "luajit", feature = "luau"))))] {
        include!("main_inner.rs");
    } else if #[cfg(all(feature = "luajit", not(any(feature = "lua55", feature = "lua54", feature = "lua53", feature = "lua52", feature = "lua51", feature = "luau"))))] {
        include!("main_inner.rs");
    } else if #[cfg(all(feature = "luau", not(any(feature = "lua55", feature = "lua54", feature = "lua53", feature = "lua52", feature = "lua51", feature = "luajit"))))] {
        include!("main_inner.rs");
    } else if #[cfg(not(any(feature = "lua55", feature = "lua54", feature = "lua53", feature = "lua52", feature = "lua51", feature = "luajit", feature = "luau")))] {
        fn main() {
            compile_error!("No Lua feature enabled. Please enable one of: lua55, lua54, lua53, lua52, lua51, luajit, luajit52, luau");
        }
    } else {
        fn main() {
            let features = [
                (cfg!(feature = "lua55"), "lua55"),
                (cfg!(feature = "lua54"), "lua54"),
                (cfg!(feature = "lua53"), "lua53"),
                (cfg!(feature = "lua52"), "lua52"),
                (cfg!(feature = "lua51"), "lua51"),
                (cfg!(feature = "luajit52"), "luajit52"),
                (
                    cfg!(all(feature = "luajit", not(feature = "luajit52"))),
                    "luajit",
                ),
                (cfg!(feature = "luau"), "luau"),
            ]
            .into_iter()
            .filter_map(|(enabled, feature)| enabled.then_some(feature))
            .collect::<Vec<_>>()
            .join(", ");

            println!("cargo::error=You enabled {features}; only one Lua feature can be enabled");
        }
    }
}

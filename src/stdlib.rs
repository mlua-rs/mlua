use std::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign, BitXor, BitXorAssign};

/// Flags describing the set of lua standard libraries to load.
#[derive(Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct StdLib(u32);

impl StdLib {
    /// [`coroutine`](https://www.lua.org/manual/5.4/manual.html#6.2) library
    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52", feature = "luau"))]
    #[cfg_attr(
        docsrs,
        doc(cfg(any(feature = "lua54", feature = "lua53", feature = "lua52", feature = "luau")))
    )]
    pub const COROUTINE: StdLib = StdLib(1);

    /// [`table`](https://www.lua.org/manual/5.4/manual.html#6.6) library
    pub const TABLE: StdLib = StdLib(1 << 1);

    /// [`io`](https://www.lua.org/manual/5.4/manual.html#6.8) library
    #[cfg(not(feature = "luau"))]
    #[cfg_attr(docsrs, doc(cfg(not(feature = "luau"))))]
    pub const IO: StdLib = StdLib(1 << 2);

    /// [`os`](https://www.lua.org/manual/5.4/manual.html#6.9) library
    pub const OS: StdLib = StdLib(1 << 3);

    /// [`string`](https://www.lua.org/manual/5.4/manual.html#6.4) library
    pub const STRING: StdLib = StdLib(1 << 4);

    /// [`utf8`](https://www.lua.org/manual/5.4/manual.html#6.5) library
    #[cfg(any(feature = "lua54", feature = "lua53", feature = "luau"))]
    #[cfg_attr(docsrs, doc(cfg(any(feature = "lua54", feature = "lua53", feature = "luau"))))]
    pub const UTF8: StdLib = StdLib(1 << 5);

    /// [`bit`](https://www.lua.org/manual/5.2/manual.html#6.7) library
    #[cfg(any(feature = "lua52", feature = "luajit", feature = "luau", doc))]
    #[cfg_attr(
        docsrs,
        doc(cfg(any(feature = "lua52", feature = "luajit", feature = "luau")))
    )]
    pub const BIT: StdLib = StdLib(1 << 6);

    /// [`math`](https://www.lua.org/manual/5.4/manual.html#6.7) library
    pub const MATH: StdLib = StdLib(1 << 7);

    /// [`package`](https://www.lua.org/manual/5.4/manual.html#6.3) library
    #[cfg(not(feature = "luau"))]
    #[cfg_attr(docsrs, doc(cfg(not(feature = "luau"))))]
    pub const PACKAGE: StdLib = StdLib(1 << 8);

    /// [`buffer`](https://luau.org/library#buffer-library) library
    #[cfg(any(feature = "luau", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub const BUFFER: StdLib = StdLib(1 << 9);

    /// [`vector`](https://luau.org/library#vector-library) library
    #[cfg(any(feature = "luau", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub const VECTOR: StdLib = StdLib(1 << 10);

    /// [`jit`](http://luajit.org/ext_jit.html) library
    #[cfg(any(feature = "luajit", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luajit")))]
    pub const JIT: StdLib = StdLib(1 << 11);

    /// (**unsafe**) [`ffi`](http://luajit.org/ext_ffi.html) library
    #[cfg(any(feature = "luajit", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luajit")))]
    pub const FFI: StdLib = StdLib(1 << 30);

    /// (**unsafe**) [`debug`](https://www.lua.org/manual/5.4/manual.html#6.10) library
    pub const DEBUG: StdLib = StdLib(1 << 31);

    /// No libraries
    pub const NONE: StdLib = StdLib(0);
    /// (**unsafe**) All standard libraries
    pub const ALL: StdLib = StdLib(u32::MAX);
    /// The safe subset of the standard libraries
    #[cfg(not(feature = "luau"))]
    pub const ALL_SAFE: StdLib = StdLib((1 << 30) - 1);
    #[cfg(feature = "luau")]
    pub const ALL_SAFE: StdLib = StdLib(u32::MAX);

    pub fn contains(self, lib: Self) -> bool {
        (self & lib).0 != 0
    }
}

impl BitAnd for StdLib {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self::Output {
        StdLib(self.0 & rhs.0)
    }
}

impl BitAndAssign for StdLib {
    fn bitand_assign(&mut self, rhs: Self) {
        *self = StdLib(self.0 & rhs.0)
    }
}

impl BitOr for StdLib {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self::Output {
        StdLib(self.0 | rhs.0)
    }
}

impl BitOrAssign for StdLib {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = StdLib(self.0 | rhs.0)
    }
}

impl BitXor for StdLib {
    type Output = Self;
    fn bitxor(self, rhs: Self) -> Self::Output {
        StdLib(self.0 ^ rhs.0)
    }
}

impl BitXorAssign for StdLib {
    fn bitxor_assign(&mut self, rhs: Self) {
        *self = StdLib(self.0 ^ rhs.0)
    }
}

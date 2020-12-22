use std::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign, BitXor, BitXorAssign};
use std::u32;

/// Flags describing the set of lua standard libraries to load.
#[derive(Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct StdLib(u32);

impl StdLib {
    /// [`coroutine`](https://www.lua.org/manual/5.3/manual.html#6.2) library
    ///
    /// Requires `feature = "lua54/lua53/lua52"`
    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52", doc))]
    pub const COROUTINE: StdLib = StdLib(1);
    /// [`table`](https://www.lua.org/manual/5.3/manual.html#6.6) library
    pub const TABLE: StdLib = StdLib(1 << 1);
    /// [`io`](https://www.lua.org/manual/5.3/manual.html#6.8) library
    pub const IO: StdLib = StdLib(1 << 2);
    /// [`os`](https://www.lua.org/manual/5.3/manual.html#6.9) library
    pub const OS: StdLib = StdLib(1 << 3);
    /// [`string`](https://www.lua.org/manual/5.3/manual.html#6.4) library
    pub const STRING: StdLib = StdLib(1 << 4);
    /// [`utf8`](https://www.lua.org/manual/5.3/manual.html#6.5) library
    ///
    /// Requires `feature = "lua54/lua53"`
    #[cfg(any(feature = "lua54", feature = "lua53", doc))]
    pub const UTF8: StdLib = StdLib(1 << 5);
    /// [`bit`](https://www.lua.org/manual/5.2/manual.html#6.7) library
    ///
    /// Requires `feature = "lua52/luajit"`
    #[cfg(any(feature = "lua52", feature = "luajit", doc))]
    pub const BIT: StdLib = StdLib(1 << 6);
    /// [`math`](https://www.lua.org/manual/5.3/manual.html#6.7) library
    pub const MATH: StdLib = StdLib(1 << 7);
    /// [`package`](https://www.lua.org/manual/5.3/manual.html#6.3) library
    pub const PACKAGE: StdLib = StdLib(1 << 8);
    /// [`jit`](http://luajit.org/ext_jit.html) library
    ///
    /// Requires `feature = "luajit"`
    #[cfg(any(feature = "luajit", doc))]
    pub const JIT: StdLib = StdLib(1 << 9);

    /// (unsafe) [`ffi`](http://luajit.org/ext_ffi.html) library
    ///
    /// Requires `feature = "luajit"`
    #[cfg(any(feature = "luajit", doc))]
    pub const FFI: StdLib = StdLib(1 << 30);
    /// (unsafe) [`debug`](https://www.lua.org/manual/5.3/manual.html#6.10) library
    pub const DEBUG: StdLib = StdLib(1 << 31);

    /// No libraries
    pub const NONE: StdLib = StdLib(0);
    /// (unsafe) All standard libraries
    pub const ALL: StdLib = StdLib(u32::MAX);
    /// The safe subset of the standard libraries
    pub const ALL_SAFE: StdLib = StdLib((1 << 30) - 1);

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

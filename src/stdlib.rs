use std::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign, BitXor, BitXorAssign};
use std::u32;

/// Flags describing the set of lua modules to load.
#[derive(Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct StdLib(u32);

impl StdLib {
    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
    pub const COROUTINE: StdLib = StdLib(1);
    pub const TABLE: StdLib = StdLib(1 << 1);
    pub const IO: StdLib = StdLib(1 << 2);
    pub const OS: StdLib = StdLib(1 << 3);
    pub const STRING: StdLib = StdLib(1 << 4);
    #[cfg(any(feature = "lua54", feature = "lua53"))]
    pub const UTF8: StdLib = StdLib(1 << 5);
    #[cfg(any(feature = "lua52", feature = "luajit"))]
    pub const BIT: StdLib = StdLib(1 << 6);
    pub const MATH: StdLib = StdLib(1 << 7);
    pub const PACKAGE: StdLib = StdLib(1 << 8);
    #[cfg(feature = "luajit")]
    pub const JIT: StdLib = StdLib(1 << 9);

    /// `ffi` (unsafe) module `feature = "luajit"`
    #[cfg(feature = "luajit")]
    pub const FFI: StdLib = StdLib(1 << 30);
    /// `debug` (unsafe) module
    pub const DEBUG: StdLib = StdLib(1 << 31);

    pub const ALL: StdLib = StdLib(u32::MAX);
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

use std::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign, BitXor, BitXorAssign};

/// Flags describing the set of lua standard libraries to load.
#[derive(Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct StdLib(u32);

impl StdLib {
    /// [`coroutine`](https://www.lua.org/manual/5.4/manual.html#6.2) library
    ///
    /// Requires `feature = "lua54/lua53/lua52/luau"`
    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52", feature = "luau"))]
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
    ///
    /// Requires `feature = "lua54/lua53/luau"`
    #[cfg(any(feature = "lua54", feature = "lua53", feature = "luau"))]
    pub const UTF8: StdLib = StdLib(1 << 5);

    /// [`bit`](https://www.lua.org/manual/5.2/manual.html#6.7) library
    ///
    /// Requires `feature = "lua52/luajit/luau"`
    #[cfg(any(feature = "lua52", feature = "luajit", feature = "luau", doc))]
    pub const BIT: StdLib = StdLib(1 << 6);

    /// [`math`](https://www.lua.org/manual/5.4/manual.html#6.7) library
    pub const MATH: StdLib = StdLib(1 << 7);

    /// [`package`](https://www.lua.org/manual/5.4/manual.html#6.3) library
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
    ///
    /// Requires `feature = "luajit"`
    #[cfg(any(feature = "luajit", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luajit")))]
    pub const JIT: StdLib = StdLib(1 << 11);

    /// (**unsafe**) FFI library
    #[cfg(any(feature = "luajit", feature = "pluto", doc))]
    #[cfg_attr(docsrs, doc(cfg(any(feature = "luajit", feature = "pluto"))))]
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

#[cfg(feature = "pluto")]
#[cfg_attr(docsrs, doc(cfg(feature = "pluto")))]
impl StdLib {
    /// Extended assertion utilities library
    pub const ASSERT: StdLib = StdLib(1 << 12);

    /// Base32 encoding/decoding library
    pub const BASE32: StdLib = StdLib(1 << 13);

    /// Base64 encoding/decoding library
    pub const BASE64: StdLib = StdLib(1 << 14);

    /// Arbitrary-precision integer arithmetic library
    pub const BIGINT: StdLib = StdLib(1 << 15);

    /// 2D graphics library
    pub const CANVAS: StdLib = StdLib(1 << 16);

    /// Encoding and decoding library for the [Colons and Tabs] format.
    ///
    /// [Colons and Tabs]: https://github.com/calamity-inc/Soup/blob/senpai/docs/user/cat.md
    pub const CAT: StdLib = StdLib(1 << 17);

    /// Cryptographic library
    pub const CRYPTO: StdLib = StdLib(1 << 18);

    /// HTTP client library
    pub const HTTP: StdLib = StdLib(1 << 19);

    /// JSON encoding/decoding library
    pub const JSON: StdLib = StdLib(1 << 20);

    /// Regular expression library
    pub const REGEX: StdLib = StdLib(1 << 21);

    /// Task scheduling library
    pub const SCHEDULER: StdLib = StdLib(1 << 22);

    /// Network socket library
    pub const SOCKET: StdLib = StdLib(1 << 23);

    /// URL parsing library
    pub const URL: StdLib = StdLib(1 << 24);

    /// 3D vector library
    pub const VECTOR3: StdLib = StdLib(1 << 25);

    /// XML encoding/decoding library
    pub const XML: StdLib = StdLib(1 << 26);
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

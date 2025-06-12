use std::fmt;

#[cfg(feature = "serde")]
use serde::ser::{Serialize, SerializeTupleStruct, Serializer};

/// A Luau vector type.
///
/// By default vectors are 3-dimensional, but can be 4-dimensional
/// if the `luau-vector4` feature is enabled.
#[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
#[derive(Debug, Default, Clone, Copy, PartialEq, PartialOrd)]
pub struct Vector(pub(crate) [f32; Self::SIZE]);

impl fmt::Display for Vector {
    #[rustfmt::skip]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        #[cfg(not(feature = "luau-vector4"))]
        return write!(f, "vector({}, {}, {})", self.x(), self.y(), self.z());
        #[cfg(feature = "luau-vector4")]
        return write!(f, "vector({}, {}, {}, {})", self.x(), self.y(), self.z(), self.w());
    }
}

#[cfg_attr(not(feature = "luau"), allow(unused))]
impl Vector {
    pub(crate) const SIZE: usize = if cfg!(feature = "luau-vector4") { 4 } else { 3 };

    /// Creates a new vector.
    #[cfg(not(feature = "luau-vector4"))]
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self([x, y, z])
    }

    /// Creates a new vector.
    #[cfg(feature = "luau-vector4")]
    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self([x, y, z, w])
    }

    /// Creates a new vector with all components set to `0.0`.
    pub const fn zero() -> Self {
        Self([0.0; Self::SIZE])
    }

    /// Returns 1st component of the vector.
    pub const fn x(&self) -> f32 {
        self.0[0]
    }

    /// Returns 2nd component of the vector.
    pub const fn y(&self) -> f32 {
        self.0[1]
    }

    /// Returns 3rd component of the vector.
    pub const fn z(&self) -> f32 {
        self.0[2]
    }

    /// Returns 4th component of the vector.
    #[cfg(any(feature = "luau-vector4", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau-vector4")))]
    pub const fn w(&self) -> f32 {
        self.0[3]
    }
}

#[cfg(feature = "serde")]
impl Serialize for Vector {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut ts = serializer.serialize_tuple_struct("Vector", Self::SIZE)?;
        ts.serialize_field(&self.x())?;
        ts.serialize_field(&self.y())?;
        ts.serialize_field(&self.z())?;
        #[cfg(feature = "luau-vector4")]
        ts.serialize_field(&self.w())?;
        ts.end()
    }
}

impl PartialEq<[f32; Self::SIZE]> for Vector {
    #[inline]
    fn eq(&self, other: &[f32; Self::SIZE]) -> bool {
        self.0 == *other
    }
}

#[cfg(feature = "luau")]
impl crate::types::LuaType for Vector {
    const TYPE_ID: std::os::raw::c_int = ffi::LUA_TVECTOR;
}

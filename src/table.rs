use std::prelude::v1::*;

use std::collections::HashSet;
use std::ffi::c_void;
use std::fmt;
use std::marker::PhantomData;

#[cfg(feature = "serialize")]
use {
    rustc_hash::FxHashSet,
    serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer},
    std::{cell::RefCell, rc::Rc, result::Result as StdResult},
};

use crate::error::{Error, Result};
use crate::function::Function;
use crate::private::Sealed;
use crate::types::{Integer, LuaRef};
use crate::util::{assert_stack, check_stack, StackGuard};
use crate::value::{FromLua, FromLuaMulti, IntoLua, IntoLuaMulti, Nil, Value};

#[cfg(feature = "async")]
use futures_util::future::{self, LocalBoxFuture};

/// Handle to an internal Lua table.
#[derive(Clone)]
pub struct Table<'lua>(pub(crate) LuaRef<'lua>);

/// Owned handle to an internal Lua table.
///
/// The owned handle holds a *strong* reference to the current Lua instance.
/// Be warned, if you place it into a Lua type (eg. [`UserData`] or a Rust callback), it is *very easy*
/// to accidentally cause reference cycles that would prevent destroying Lua instance.
///
/// [`UserData`]: crate::UserData
#[cfg(feature = "unstable")]
#[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
#[derive(Clone, Debug)]
pub struct OwnedTable(pub(crate) crate::types::LuaOwnedRef);

#[cfg(feature = "unstable")]
impl OwnedTable {
    /// Get borrowed handle to the underlying Lua table.
    #[cfg_attr(feature = "send", allow(unused))]
    pub const fn to_ref(&self) -> Table {
        Table(self.0.to_ref())
    }
}

impl<'lua> Table<'lua> {
    /// Sets a key-value pair in the table.
    ///
    /// If the value is `nil`, this will effectively remove the pair.
    ///
    /// This might invoke the `__newindex` metamethod. Use the [`raw_set`] method if that is not
    /// desired.
    ///
    /// # Examples
    ///
    /// Export a value as a global to make it usable from Lua:
    ///
    /// ```
    /// # use mlua::{Lua, Result};
    /// # fn main() -> Result<()> {
    /// # let lua = Lua::new();
    /// let globals = lua.globals();
    ///
    /// globals.set("assertions", cfg!(debug_assertions))?;
    ///
    /// lua.load(r#"
    ///     if assertions == true then
    ///         -- ...
    ///     elseif assertions == false then
    ///         -- ...
    ///     else
    ///         error("assertions neither on nor off?")
    ///     end
    /// "#).exec()?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// [`raw_set`]: #method.raw_set
    pub fn set<K: IntoLua<'lua>, V: IntoLua<'lua>>(&self, key: K, value: V) -> Result<()> {
        // Fast track
        if !self.has_metatable() {
            return self.raw_set(key, value);
        }

        let lua = self.0.lua;
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 5)?;

            lua.push_ref(&self.0);
            key.push_into_stack(lua)?;
            value.push_into_stack(lua)?;
            protect_lua!(state, 3, 0, fn(state) ffi::lua_settable(state, -3))
        }
    }

    /// Gets the value associated to `key` from the table.
    ///
    /// If no value is associated to `key`, returns the `nil` value.
    ///
    /// This might invoke the `__index` metamethod. Use the [`raw_get`] method if that is not
    /// desired.
    ///
    /// # Examples
    ///
    /// Query the version of the Lua interpreter:
    ///
    /// ```
    /// # use mlua::{Lua, Result};
    /// # fn main() -> Result<()> {
    /// # let lua = Lua::new();
    /// let globals = lua.globals();
    ///
    /// let version: String = globals.get("_VERSION")?;
    /// println!("Lua version: {}", version);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// [`raw_get`]: #method.raw_get
    pub fn get<K: IntoLua<'lua>, V: FromLua<'lua>>(&self, key: K) -> Result<V> {
        // Fast track
        if !self.has_metatable() {
            return self.raw_get(key);
        }

        let lua = self.0.lua;
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 4)?;

            lua.push_ref(&self.0);
            key.push_into_stack(lua)?;
            protect_lua!(state, 2, 1, fn(state) ffi::lua_gettable(state, -2))?;

            V::from_stack(-1, lua)
        }
    }

    /// Checks whether the table contains a non-nil value for `key`.
    ///
    /// This might invoke the `__index` metamethod.
    pub fn contains_key<K: IntoLua<'lua>>(&self, key: K) -> Result<bool> {
        Ok(self.get::<_, Value>(key)? != Value::Nil)
    }

    /// Appends a value to the back of the table.
    ///
    /// This might invoke the `__len` and `__newindex` metamethods.
    pub fn push<V: IntoLua<'lua>>(&self, value: V) -> Result<()> {
        // Fast track
        if !self.has_metatable() {
            return self.raw_push(value);
        }

        let lua = self.0.lua;
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 4)?;

            lua.push_ref(&self.0);
            value.push_into_stack(lua)?;
            protect_lua!(state, 2, 0, fn(state) {
                let len = ffi::luaL_len(state, -2) as Integer;
                ffi::lua_seti(state, -2, len + 1);
            })?
        }
        Ok(())
    }

    /// Removes the last element from the table and returns it.
    ///
    /// This might invoke the `__len` and `__newindex` metamethods.
    pub fn pop<V: FromLua<'lua>>(&self) -> Result<V> {
        // Fast track
        if !self.has_metatable() {
            return self.raw_pop();
        }

        let lua = self.0.lua;
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 4)?;

            lua.push_ref(&self.0);
            protect_lua!(state, 1, 1, fn(state) {
                let len = ffi::luaL_len(state, -1) as Integer;
                ffi::lua_geti(state, -1, len);
                ffi::lua_pushnil(state);
                ffi::lua_seti(state, -3, len);
            })?;
            V::from_stack(-1, lua)
        }
    }

    /// Compares two tables for equality.
    ///
    /// Tables are compared by reference first.
    /// If they are not primitively equals, then mlua will try to invoke the `__eq` metamethod.
    /// mlua will check `self` first for the metamethod, then `other` if not found.
    ///
    /// # Examples
    ///
    /// Compare two tables using `__eq` metamethod:
    ///
    /// ```
    /// # use mlua::{Lua, Result, Table};
    /// # fn main() -> Result<()> {
    /// # let lua = Lua::new();
    /// let table1 = lua.create_table()?;
    /// table1.set(1, "value")?;
    ///
    /// let table2 = lua.create_table()?;
    /// table2.set(2, "value")?;
    ///
    /// let always_equals_mt = lua.create_table()?;
    /// always_equals_mt.set("__eq", lua.create_function(|_, (_t1, _t2): (Table, Table)| Ok(true))?)?;
    /// table2.set_metatable(Some(always_equals_mt));
    ///
    /// assert!(table1.equals(&table1.clone())?);
    /// assert!(table1.equals(&table2)?);
    /// # Ok(())
    /// # }
    /// ```
    pub fn equals<T: AsRef<Self>>(&self, other: T) -> Result<bool> {
        let other = other.as_ref();
        if self == other {
            return Ok(true);
        }

        // Compare using __eq metamethod if exists
        // First, check the self for the metamethod.
        // If self does not define it, then check the other table.
        if let Some(mt) = self.get_metatable() {
            if mt.contains_key("__eq")? {
                return mt
                    .get::<_, Function>("__eq")?
                    .call((self.clone(), other.clone()));
            }
        }
        if let Some(mt) = other.get_metatable() {
            if mt.contains_key("__eq")? {
                return mt
                    .get::<_, Function>("__eq")?
                    .call((self.clone(), other.clone()));
            }
        }

        Ok(false)
    }

    /// Sets a key-value pair without invoking metamethods.
    pub fn raw_set<K: IntoLua<'lua>, V: IntoLua<'lua>>(&self, key: K, value: V) -> Result<()> {
        #[cfg(feature = "luau")]
        self.check_readonly_write()?;

        let lua = self.0.lua;
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 5)?;

            lua.push_ref(&self.0);
            key.push_into_stack(lua)?;
            value.push_into_stack(lua)?;

            if lua.unlikely_memory_error() {
                ffi::lua_rawset(state, -3);
                ffi::lua_pop(state, 1);
                Ok(())
            } else {
                protect_lua!(state, 3, 0, fn(state) ffi::lua_rawset(state, -3))
            }
        }
    }

    /// Gets the value associated to `key` without invoking metamethods.
    pub fn raw_get<K: IntoLua<'lua>, V: FromLua<'lua>>(&self, key: K) -> Result<V> {
        let lua = self.0.lua;
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 3)?;

            lua.push_ref(&self.0);
            key.push_into_stack(lua)?;
            ffi::lua_rawget(state, -2);

            V::from_stack(-1, lua)
        }
    }

    /// Inserts element value at position `idx` to the table, shifting up the elements from `table[idx]`.
    /// The worst case complexity is O(n), where n is the table length.
    pub fn raw_insert<V: IntoLua<'lua>>(&self, idx: Integer, value: V) -> Result<()> {
        let size = self.raw_len() as Integer;
        if idx < 1 || idx > size + 1 {
            return Err(Error::runtime("index out of bounds"));
        }

        let lua = self.0.lua;
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 5)?;

            lua.push_ref(&self.0);
            value.push_into_stack(lua)?;
            protect_lua!(state, 2, 0, |state| {
                for i in (idx..=size).rev() {
                    // table[i+1] = table[i]
                    ffi::lua_rawgeti(state, -2, i);
                    ffi::lua_rawseti(state, -3, i + 1);
                }
                ffi::lua_rawseti(state, -2, idx)
            })
        }
    }

    /// Appends a value to the back of the table without invoking metamethods.
    pub fn raw_push<V: IntoLua<'lua>>(&self, value: V) -> Result<()> {
        #[cfg(feature = "luau")]
        self.check_readonly_write()?;

        let lua = self.0.lua;
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 4)?;

            lua.push_ref(&self.0);
            value.push_into_stack(lua)?;

            unsafe fn callback(state: *mut ffi::lua_State) {
                let len = ffi::lua_rawlen(state, -2) as Integer;
                ffi::lua_rawseti(state, -2, len + 1);
            }

            if lua.unlikely_memory_error() {
                callback(state);
            } else {
                protect_lua!(state, 2, 0, fn(state) callback(state))?;
            }
        }
        Ok(())
    }

    /// Removes the last element from the table and returns it, without invoking metamethods.
    pub fn raw_pop<V: FromLua<'lua>>(&self) -> Result<V> {
        #[cfg(feature = "luau")]
        self.check_readonly_write()?;

        let lua = self.0.lua;
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 3)?;

            lua.push_ref(&self.0);
            let len = ffi::lua_rawlen(state, -1) as Integer;
            ffi::lua_rawgeti(state, -1, len);
            // Set slot to nil (it must be safe to do)
            ffi::lua_pushnil(state);
            ffi::lua_rawseti(state, -3, len);

            V::from_stack(-1, lua)
        }
    }

    /// Removes a key from the table.
    ///
    /// If `key` is an integer, mlua shifts down the elements from `table[key+1]`,
    /// and erases element `table[key]`. The complexity is O(n) in the worst case,
    /// where n is the table length.
    ///
    /// For other key types this is equivalent to setting `table[key] = nil`.
    pub fn raw_remove<K: IntoLua<'lua>>(&self, key: K) -> Result<()> {
        let lua = self.0.lua;
        let state = lua.state();
        let key = key.into_lua(lua)?;
        match key {
            Value::Integer(idx) => {
                let size = self.raw_len() as Integer;
                if idx < 1 || idx > size {
                    return Err(Error::runtime("index out of bounds"));
                }
                unsafe {
                    let _sg = StackGuard::new(state);
                    check_stack(state, 4)?;

                    lua.push_ref(&self.0);
                    protect_lua!(state, 1, 0, |state| {
                        for i in idx..size {
                            ffi::lua_rawgeti(state, -1, i + 1);
                            ffi::lua_rawseti(state, -2, i);
                        }
                        ffi::lua_pushnil(state);
                        ffi::lua_rawseti(state, -2, size);
                    })
                }
            }
            _ => self.raw_set(key, Nil),
        }
    }

    /// Clears the table, removing all keys and values from array and hash parts,
    /// without invoking metamethods.
    ///
    /// This method is useful to clear the table while keeping its capacity.
    pub fn clear(&self) -> Result<()> {
        #[cfg(feature = "luau")]
        self.check_readonly_write()?;

        let lua = self.0.lua;
        unsafe {
            #[cfg(feature = "luau")]
            ffi::lua_cleartable(lua.ref_thread(), self.0.index);

            #[cfg(not(feature = "luau"))]
            {
                let state = lua.state();
                check_stack(state, 4)?;

                lua.push_ref(&self.0);

                // Clear array part
                for i in 1..=ffi::lua_rawlen(state, -1) {
                    ffi::lua_pushnil(state);
                    ffi::lua_rawseti(state, -2, i as Integer);
                }

                // Clear hash part
                // It must be safe as long as we don't use invalid keys
                ffi::lua_pushnil(state);
                while ffi::lua_next(state, -2) != 0 {
                    ffi::lua_pop(state, 1); // pop value
                    ffi::lua_pushvalue(state, -1); // copy key
                    ffi::lua_pushnil(state);
                    ffi::lua_rawset(state, -4);
                }
            }
        }

        Ok(())
    }

    /// Returns the result of the Lua `#` operator.
    ///
    /// This might invoke the `__len` metamethod. Use the [`raw_len`] method if that is not desired.
    ///
    /// [`raw_len`]: #method.raw_len
    pub fn len(&self) -> Result<Integer> {
        // Fast track
        if !self.has_metatable() {
            return Ok(self.raw_len() as Integer);
        }

        let lua = self.0.lua;
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 4)?;

            lua.push_ref(&self.0);
            protect_lua!(state, 1, 0, |state| ffi::luaL_len(state, -1))
        }
    }

    /// Returns the result of the Lua `#` operator, without invoking the `__len` metamethod.
    pub fn raw_len(&self) -> usize {
        let ref_thread = self.0.lua.ref_thread();
        unsafe { ffi::lua_rawlen(ref_thread, self.0.index) }
    }

    /// Returns `true` if the table is empty, without invoking metamethods.
    ///
    /// It checks both the array part and the hash part.
    pub fn is_empty(&self) -> bool {
        // Check array part
        if self.raw_len() != 0 {
            return false;
        }

        // Check hash part
        let lua = self.0.lua;
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            assert_stack(state, 4);

            lua.push_ref(&self.0);
            ffi::lua_pushnil(state);
            if ffi::lua_next(state, -2) != 0 {
                return false;
            }
        }

        true
    }

    /// Returns a reference to the metatable of this table, or `None` if no metatable is set.
    ///
    /// Unlike the `getmetatable` Lua function, this method ignores the `__metatable` field.
    pub fn get_metatable(&self) -> Option<Table<'lua>> {
        let lua = self.0.lua;
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            assert_stack(state, 2);

            lua.push_ref(&self.0);
            if ffi::lua_getmetatable(state, -1) == 0 {
                None
            } else {
                Some(Table(lua.pop_ref()))
            }
        }
    }

    /// Sets or removes the metatable of this table.
    ///
    /// If `metatable` is `None`, the metatable is removed (if no metatable is set, this does
    /// nothing).
    pub fn set_metatable(&self, metatable: Option<Table<'lua>>) {
        // Workaround to throw readonly error without returning Result
        #[cfg(feature = "luau")]
        if self.is_readonly() {
            panic!("attempt to modify a readonly table");
        }

        let lua = self.0.lua;
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            assert_stack(state, 2);

            lua.push_ref(&self.0);
            if let Some(metatable) = metatable {
                lua.push_ref(&metatable.0);
            } else {
                ffi::lua_pushnil(state);
            }
            ffi::lua_setmetatable(state, -2);
        }
    }

    /// Returns true if the table has metatable attached.
    #[doc(hidden)]
    #[inline]
    pub fn has_metatable(&self) -> bool {
        let ref_thread = self.0.lua.ref_thread();
        unsafe {
            if ffi::lua_getmetatable(ref_thread, self.0.index) != 0 {
                ffi::lua_pop(ref_thread, 1);
                return true;
            }
        }
        false
    }

    /// Sets `readonly` attribute on the table.
    ///
    /// Requires `feature = "luau"`
    #[cfg(any(feature = "luau", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub fn set_readonly(&self, enabled: bool) {
        let ref_thread = self.0.lua.ref_thread();
        unsafe {
            ffi::lua_setreadonly(ref_thread, self.0.index, enabled as _);
            if !enabled {
                // Reset "safeenv" flag
                ffi::lua_setsafeenv(ref_thread, self.0.index, 0);
            }
        }
    }

    /// Returns `readonly` attribute of the table.
    ///
    /// Requires `feature = "luau"`
    #[cfg(any(feature = "luau", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub fn is_readonly(&self) -> bool {
        let ref_thread = self.0.lua.ref_thread();
        unsafe { ffi::lua_getreadonly(ref_thread, self.0.index) != 0 }
    }

    /// Converts the table to a generic C pointer.
    ///
    /// Different tables will give different pointers.
    /// There is no way to convert the pointer back to its original value.
    ///
    /// Typically this function is used only for hashing and debug information.
    #[inline]
    pub fn to_pointer(&self) -> *const c_void {
        self.0.to_pointer()
    }

    /// Convert this handle to owned version.
    #[cfg(all(feature = "unstable", any(not(feature = "send"), doc)))]
    #[cfg_attr(docsrs, doc(cfg(all(feature = "unstable", not(feature = "send")))))]
    #[inline]
    pub fn into_owned(self) -> OwnedTable {
        OwnedTable(self.0.into_owned())
    }

    /// Consume this table and return an iterator over the pairs of the table.
    ///
    /// This works like the Lua `pairs` function, but does not invoke the `__pairs` metamethod.
    ///
    /// The pairs are wrapped in a [`Result`], since they are lazily converted to `K` and `V` types.
    ///
    /// # Note
    ///
    /// While this method consumes the `Table` object, it can not prevent code from mutating the
    /// table while the iteration is in progress. Refer to the [Lua manual] for information about
    /// the consequences of such mutation.
    ///
    /// # Examples
    ///
    /// Iterate over all globals:
    ///
    /// ```
    /// # use mlua::{Lua, Result, Value};
    /// # fn main() -> Result<()> {
    /// # let lua = Lua::new();
    /// let globals = lua.globals();
    ///
    /// for pair in globals.pairs::<Value, Value>() {
    ///     let (key, value) = pair?;
    /// #   let _ = (key, value);   // used
    ///     // ...
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// [`Result`]: crate::Result
    /// [Lua manual]: http://www.lua.org/manual/5.4/manual.html#pdf-next
    pub fn pairs<K: FromLua<'lua>, V: FromLua<'lua>>(self) -> TablePairs<'lua, K, V> {
        TablePairs {
            table: self.0,
            key: Some(Nil),
            _phantom: PhantomData,
        }
    }

    /// Consume this table and return an iterator over all values in the sequence part of the table.
    ///
    /// The iterator will yield all values `t[1]`, `t[2]` and so on, until a `nil` value is
    /// encountered. This mirrors the behavior of Lua's `ipairs` function but does not invoke
    /// any metamethods.
    ///
    /// # Note
    ///
    /// While this method consumes the `Table` object, it can not prevent code from mutating the
    /// table while the iteration is in progress. Refer to the [Lua manual] for information about
    /// the consequences of such mutation.
    ///
    /// # Examples
    ///
    /// ```
    /// # use mlua::{Lua, Result, Table};
    /// # fn main() -> Result<()> {
    /// # let lua = Lua::new();
    /// let my_table: Table = lua.load(r#"
    ///     {
    ///         [1] = 4,
    ///         [2] = 5,
    ///         [4] = 7,
    ///         key = 2
    ///     }
    /// "#).eval()?;
    ///
    /// let expected = [4, 5];
    /// for (&expected, got) in expected.iter().zip(my_table.sequence_values::<u32>()) {
    ///     assert_eq!(expected, got?);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// [`pairs`]: #method.pairs
    /// [`Result`]: crate::Result
    /// [Lua manual]: http://www.lua.org/manual/5.4/manual.html#pdf-next
    pub fn sequence_values<V: FromLua<'lua>>(self) -> TableSequence<'lua, V> {
        TableSequence {
            table: self.0,
            index: Some(1),
            len: None,
            _phantom: PhantomData,
        }
    }

    #[doc(hidden)]
    #[deprecated(since = "0.9.0", note = "use `sequence_values` instead")]
    pub fn raw_sequence_values<V: FromLua<'lua>>(self) -> TableSequence<'lua, V> {
        self.sequence_values()
    }

    #[cfg(feature = "serialize")]
    pub(crate) fn sequence_values_by_len<V: FromLua<'lua>>(
        self,
        len: Option<usize>,
    ) -> TableSequence<'lua, V> {
        let len = len.unwrap_or_else(|| self.raw_len()) as Integer;
        TableSequence {
            table: self.0,
            index: Some(1),
            len: Some(len),
            _phantom: PhantomData,
        }
    }

    /// Sets element value at position `idx` without invoking metamethods.
    #[allow(dead_code)]
    pub(crate) fn raw_seti<V: IntoLua<'lua>>(&self, idx: usize, value: V) -> Result<()> {
        #[cfg(feature = "luau")]
        self.check_readonly_write()?;

        let lua = self.0.lua;
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 5)?;

            lua.push_ref(&self.0);
            value.push_into_stack(lua)?;

            let idx = idx.try_into().unwrap();
            if lua.unlikely_memory_error() {
                ffi::lua_rawseti(state, -2, idx);
            } else {
                protect_lua!(state, 2, 0, |state| ffi::lua_rawseti(state, -2, idx))?;
            }
        }
        Ok(())
    }

    #[cfg(feature = "serialize")]
    pub(crate) fn is_array(&self) -> bool {
        let lua = self.0.lua;
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            assert_stack(state, 3);

            lua.push_ref(&self.0);
            if ffi::lua_getmetatable(state, -1) == 0 {
                return false;
            }
            crate::serde::push_array_metatable(state);
            ffi::lua_rawequal(state, -1, -2) != 0
        }
    }

    #[cfg(feature = "luau")]
    #[inline(always)]
    pub(crate) fn check_readonly_write(&self) -> Result<()> {
        if self.is_readonly() {
            return Err(Error::runtime("attempt to modify a readonly table"));
        }
        Ok(())
    }

    pub(crate) fn fmt_pretty(
        &self,
        fmt: &mut fmt::Formatter,
        ident: usize,
        visited: &mut HashSet<*const c_void>,
    ) -> fmt::Result {
        visited.insert(self.to_pointer());

        let t = self.clone();
        // Collect key/value pairs into a vector so we can sort them
        let mut pairs = t.pairs::<Value, Value>().flatten().collect::<Vec<_>>();
        // Sort keys
        pairs.sort_by(|(a, _), (b, _)| a.cmp(b));
        if pairs.is_empty() {
            return write!(fmt, "{{}}");
        }
        writeln!(fmt, "{{")?;
        for (key, value) in pairs {
            write!(fmt, "{}[", " ".repeat(ident + 2))?;
            key.fmt_pretty(fmt, false, ident + 2, visited)?;
            write!(fmt, "] = ")?;
            value.fmt_pretty(fmt, true, ident + 2, visited)?;
            writeln!(fmt, ",")?;
        }
        write!(fmt, "{}}}", " ".repeat(ident))
    }
}

impl fmt::Debug for Table<'_> {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        if fmt.alternate() {
            return self.fmt_pretty(fmt, 0, &mut HashSet::new());
        }
        fmt.write_fmt(format_args!("Table({:?})", self.0))
    }
}

impl<'lua> PartialEq for Table<'lua> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<'lua> AsRef<Table<'lua>> for Table<'lua> {
    #[inline]
    fn as_ref(&self) -> &Self {
        self
    }
}

impl<'lua, T> PartialEq<[T]> for Table<'lua>
where
    T: IntoLua<'lua> + Clone,
{
    fn eq(&self, other: &[T]) -> bool {
        let lua = self.0.lua;
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            assert_stack(state, 4);

            lua.push_ref(&self.0);

            let len = ffi::lua_rawlen(state, -1);
            for i in 0..len {
                ffi::lua_rawgeti(state, -1, (i + 1) as _);
                let val = lua.pop_value();
                if val == Nil {
                    return i == other.len();
                }
                match other.get(i).map(|v| v.clone().into_lua(lua)) {
                    Some(Ok(other_val)) if val == other_val => continue,
                    _ => return false,
                }
            }
        }
        true
    }
}

impl<'lua, T> PartialEq<&[T]> for Table<'lua>
where
    T: IntoLua<'lua> + Clone,
{
    #[inline]
    fn eq(&self, other: &&[T]) -> bool {
        self == *other
    }
}

impl<'lua, T, const N: usize> PartialEq<[T; N]> for Table<'lua>
where
    T: IntoLua<'lua> + Clone,
{
    #[inline]
    fn eq(&self, other: &[T; N]) -> bool {
        self == &other[..]
    }
}

/// An extension trait for `Table`s that provides a variety of convenient functionality.
pub trait TableExt<'lua>: Sealed {
    /// Calls the table as function assuming it has `__call` metamethod.
    ///
    /// The metamethod is called with the table as its first argument, followed by the passed arguments.
    fn call<A, R>(&self, args: A) -> Result<R>
    where
        A: IntoLuaMulti<'lua>,
        R: FromLuaMulti<'lua>;

    /// Asynchronously calls the table as function assuming it has `__call` metamethod.
    ///
    /// The metamethod is called with the table as its first argument, followed by the passed arguments.
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    fn call_async<A, R>(&self, args: A) -> LocalBoxFuture<'lua, Result<R>>
    where
        A: IntoLuaMulti<'lua>,
        R: FromLuaMulti<'lua> + 'lua;

    /// Gets the function associated to `key` from the table and executes it,
    /// passing the table itself along with `args` as function arguments.
    ///
    /// This is a shortcut for
    /// `table.get::<_, Function>(key)?.call((table.clone(), arg1, ..., argN))`
    ///
    /// This might invoke the `__index` metamethod.
    fn call_method<A, R>(&self, name: &str, args: A) -> Result<R>
    where
        A: IntoLuaMulti<'lua>,
        R: FromLuaMulti<'lua>;

    /// Gets the function associated to `key` from the table and executes it,
    /// passing `args` as function arguments.
    ///
    /// This is a shortcut for
    /// `table.get::<_, Function>(key)?.call(args)`
    ///
    /// This might invoke the `__index` metamethod.
    fn call_function<A, R>(&self, name: &str, args: A) -> Result<R>
    where
        A: IntoLuaMulti<'lua>,
        R: FromLuaMulti<'lua>;

    /// Gets the function associated to `key` from the table and asynchronously executes it,
    /// passing the table itself along with `args` as function arguments and returning Future.
    ///
    /// Requires `feature = "async"`
    ///
    /// This might invoke the `__index` metamethod.
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    fn call_async_method<A, R>(&self, name: &str, args: A) -> LocalBoxFuture<'lua, Result<R>>
    where
        A: IntoLuaMulti<'lua>,
        R: FromLuaMulti<'lua> + 'lua;

    /// Gets the function associated to `key` from the table and asynchronously executes it,
    /// passing `args` as function arguments and returning Future.
    ///
    /// Requires `feature = "async"`
    ///
    /// This might invoke the `__index` metamethod.
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    fn call_async_function<A, R>(&self, name: &str, args: A) -> LocalBoxFuture<'lua, Result<R>>
    where
        A: IntoLuaMulti<'lua>,
        R: FromLuaMulti<'lua> + 'lua;
}

impl<'lua> TableExt<'lua> for Table<'lua> {
    fn call<A, R>(&self, args: A) -> Result<R>
    where
        A: IntoLuaMulti<'lua>,
        R: FromLuaMulti<'lua>,
    {
        // Convert table to a function and call via pcall that respects the `__call` metamethod.
        Function(self.0.clone()).call(args)
    }

    #[cfg(feature = "async")]
    fn call_async<A, R>(&self, args: A) -> LocalBoxFuture<'lua, Result<R>>
    where
        A: IntoLuaMulti<'lua>,
        R: FromLuaMulti<'lua> + 'lua,
    {
        let args = match args.into_lua_multi(self.0.lua) {
            Ok(args) => args,
            Err(e) => return Box::pin(future::err(e)),
        };
        let func = Function(self.0.clone());
        Box::pin(async move { func.call_async(args).await })
    }

    fn call_method<A, R>(&self, name: &str, args: A) -> Result<R>
    where
        A: IntoLuaMulti<'lua>,
        R: FromLuaMulti<'lua>,
    {
        let lua = self.0.lua;
        let mut args = args.into_lua_multi(lua)?;
        args.push_front(Value::Table(self.clone()));
        self.get::<_, Function>(name)?.call(args)
    }

    fn call_function<A, R>(&self, name: &str, args: A) -> Result<R>
    where
        A: IntoLuaMulti<'lua>,
        R: FromLuaMulti<'lua>,
    {
        self.get::<_, Function>(name)?.call(args)
    }

    #[cfg(feature = "async")]
    fn call_async_method<A, R>(&self, name: &str, args: A) -> LocalBoxFuture<'lua, Result<R>>
    where
        A: IntoLuaMulti<'lua>,
        R: FromLuaMulti<'lua> + 'lua,
    {
        let lua = self.0.lua;
        let mut args = match args.into_lua_multi(lua) {
            Ok(args) => args,
            Err(e) => return Box::pin(future::err(e)),
        };
        args.push_front(Value::Table(self.clone()));
        self.call_async_function(name, args)
    }

    #[cfg(feature = "async")]
    fn call_async_function<A, R>(&self, name: &str, args: A) -> LocalBoxFuture<'lua, Result<R>>
    where
        A: IntoLuaMulti<'lua>,
        R: FromLuaMulti<'lua> + 'lua,
    {
        let lua = self.0.lua;
        let args = match args.into_lua_multi(lua) {
            Ok(args) => args,
            Err(e) => return Box::pin(future::err(e)),
        };
        match self.get::<_, Function>(name) {
            Ok(func) => Box::pin(async move { func.call_async(args).await }),
            Err(e) => Box::pin(future::err(e)),
        }
    }
}

/// A wrapped [`Table`] with customized serialization behavior.
#[cfg(feature = "serialize")]
pub(crate) struct SerializableTable<'a, 'lua> {
    table: &'a Table<'lua>,
    options: crate::serde::de::Options,
    visited: Rc<RefCell<FxHashSet<*const c_void>>>,
}

#[cfg(feature = "serialize")]
impl<'lua> Serialize for Table<'lua> {
    #[inline]
    fn serialize<S: Serializer>(&self, serializer: S) -> StdResult<S::Ok, S::Error> {
        SerializableTable::new(self, Default::default(), Default::default()).serialize(serializer)
    }
}

#[cfg(feature = "serialize")]
impl<'a, 'lua> SerializableTable<'a, 'lua> {
    #[inline]
    pub(crate) fn new(
        table: &'a Table<'lua>,
        options: crate::serde::de::Options,
        visited: Rc<RefCell<FxHashSet<*const c_void>>>,
    ) -> Self {
        Self {
            table,
            options,
            visited,
        }
    }
}

#[cfg(feature = "serialize")]
impl<'a, 'lua> Serialize for SerializableTable<'a, 'lua> {
    fn serialize<S>(&self, serializer: S) -> StdResult<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use crate::serde::de::{check_value_for_skip, MapPairs};
        use crate::value::SerializableValue;

        let options = self.options;
        let visited = &self.visited;
        visited.borrow_mut().insert(self.table.to_pointer());

        // Array
        let len = self.table.raw_len();
        if len > 0 || self.table.is_array() {
            let mut seq = serializer.serialize_seq(Some(len))?;
            for value in self.table.clone().sequence_values_by_len::<Value>(None) {
                let value = &value.map_err(serde::ser::Error::custom)?;
                let skip = check_value_for_skip(value, self.options, &self.visited)
                    .map_err(serde::ser::Error::custom)?;
                if skip {
                    continue;
                }
                seq.serialize_element(&SerializableValue::new(value, options, Some(visited)))?;
            }
            return seq.end();
        }

        // HashMap
        let mut map = serializer.serialize_map(None)?;
        let pairs = MapPairs::new(self.table.clone(), self.options.sort_keys)
            .map_err(serde::ser::Error::custom)?;
        for kv in pairs {
            let (key, value) = kv.map_err(serde::ser::Error::custom)?;
            let skip_key = check_value_for_skip(&key, self.options, &self.visited)
                .map_err(serde::ser::Error::custom)?;
            let skip_value = check_value_for_skip(&value, self.options, &self.visited)
                .map_err(serde::ser::Error::custom)?;
            if skip_key || skip_value {
                continue;
            }
            map.serialize_entry(
                &SerializableValue::new(&key, options, Some(visited)),
                &SerializableValue::new(&value, options, Some(visited)),
            )?;
        }
        map.end()
    }
}

/// An iterator over the pairs of a Lua table.
///
/// This struct is created by the [`Table::pairs`] method.
///
/// [`Table::pairs`]: crate::Table::pairs
pub struct TablePairs<'lua, K, V> {
    table: LuaRef<'lua>,
    key: Option<Value<'lua>>,
    _phantom: PhantomData<(K, V)>,
}

impl<'lua, K, V> Iterator for TablePairs<'lua, K, V>
where
    K: FromLua<'lua>,
    V: FromLua<'lua>,
{
    type Item = Result<(K, V)>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(prev_key) = self.key.take() {
            let lua = self.table.lua;
            let state = lua.state();

            let res = (|| unsafe {
                let _sg = StackGuard::new(state);
                check_stack(state, 5)?;

                lua.push_ref(&self.table);
                lua.push_value(prev_key)?;

                let next = protect_lua!(state, 2, ffi::LUA_MULTRET, |state| {
                    ffi::lua_next(state, -2)
                })?;
                if next != 0 {
                    let value = lua.pop_value();
                    let key = lua.pop_value();
                    Ok(Some((
                        key.clone(),
                        K::from_lua(key, lua)?,
                        V::from_lua(value, lua)?,
                    )))
                } else {
                    Ok(None)
                }
            })();

            match res {
                Ok(Some((key, ret_key, value))) => {
                    self.key = Some(key);
                    Some(Ok((ret_key, value)))
                }
                Ok(None) => None,
                Err(e) => Some(Err(e)),
            }
        } else {
            None
        }
    }
}

/// An iterator over the sequence part of a Lua table.
///
/// This struct is created by the [`Table::sequence_values`] method.
///
/// [`Table::sequence_values`]: crate::Table::sequence_values
pub struct TableSequence<'lua, V> {
    table: LuaRef<'lua>,
    index: Option<Integer>,
    len: Option<Integer>,
    _phantom: PhantomData<V>,
}

impl<'lua, V> Iterator for TableSequence<'lua, V>
where
    V: FromLua<'lua>,
{
    type Item = Result<V>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(index) = self.index.take() {
            let lua = self.table.lua;
            let state = lua.state();

            let res = (|| unsafe {
                let _sg = StackGuard::new(state);
                check_stack(state, 1)?;

                lua.push_ref(&self.table);
                match ffi::lua_rawgeti(state, -1, index) {
                    ffi::LUA_TNIL if index > self.len.unwrap_or(0) => Ok(None),
                    _ => Ok(Some((index, lua.pop_value()))),
                }
            })();

            match res {
                Ok(Some((index, r))) => {
                    self.index = Some(index + 1);
                    Some(V::from_lua(r, lua))
                }
                Ok(None) => None,
                Err(err) => Some(Err(err)),
            }
        } else {
            None
        }
    }
}

#[cfg(test)]
mod assertions {
    use super::*;

    static_assertions::assert_not_impl_any!(Table: Send);

    #[cfg(feature = "unstable")]
    static_assertions::assert_not_impl_any!(OwnedTable: Send);
}

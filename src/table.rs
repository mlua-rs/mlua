use std::collections::HashSet;
use std::fmt;
use std::marker::PhantomData;
use std::os::raw::{c_int, c_void};
use std::string::String as StdString;

use crate::error::{Error, Result};
use crate::function::Function;
use crate::state::{LuaGuard, RawLua};
use crate::traits::{FromLua, FromLuaMulti, IntoLua, IntoLuaMulti, ObjectLike};
use crate::types::{Integer, LuaType, ValueRef};
use crate::util::{assert_stack, check_stack, get_metatable_ptr, StackGuard};
use crate::value::{Nil, Value};

#[cfg(feature = "async")]
use crate::function::AsyncCallFuture;

#[cfg(feature = "serde")]
use {
    rustc_hash::FxHashSet,
    serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer},
    std::{cell::RefCell, rc::Rc, result::Result as StdResult},
};

/// Handle to an internal Lua table.
#[derive(Clone, PartialEq)]
pub struct Table(pub(crate) ValueRef);

impl Table {
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
    /// [`raw_set`]: Table::raw_set
    pub fn set(&self, key: impl IntoLua, value: impl IntoLua) -> Result<()> {
        // Fast track (skip protected call)
        if !self.has_metatable() {
            return self.raw_set(key, value);
        }

        self.set_protected(key, value)
    }

    pub(crate) fn set_protected(&self, key: impl IntoLua, value: impl IntoLua) -> Result<()> {
        let lua = self.0.lua.lock();
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 5)?;

            lua.push_ref(&self.0);
            key.push_into_stack(&lua)?;
            value.push_into_stack(&lua)?;
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
    /// [`raw_get`]: Table::raw_get
    pub fn get<V: FromLua>(&self, key: impl IntoLua) -> Result<V> {
        // Fast track (skip protected call)
        if !self.has_metatable() {
            return self.raw_get(key);
        }

        self.get_protected(key)
    }

    pub(crate) fn get_protected<V: FromLua>(&self, key: impl IntoLua) -> Result<V> {
        let lua = self.0.lua.lock();
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 4)?;

            lua.push_ref(&self.0);
            key.push_into_stack(&lua)?;
            protect_lua!(state, 2, 1, fn(state) ffi::lua_gettable(state, -2))?;

            V::from_stack(-1, &lua)
        }
    }

    /// Checks whether the table contains a non-nil value for `key`.
    ///
    /// This might invoke the `__index` metamethod.
    pub fn contains_key(&self, key: impl IntoLua) -> Result<bool> {
        Ok(self.get::<Value>(key)? != Value::Nil)
    }

    /// Appends a value to the back of the table.
    ///
    /// This might invoke the `__len` and `__newindex` metamethods.
    pub fn push(&self, value: impl IntoLua) -> Result<()> {
        // Fast track (skip protected call)
        if !self.has_metatable() {
            return self.raw_push(value);
        }

        let lua = self.0.lua.lock();
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 4)?;

            lua.push_ref(&self.0);
            value.push_into_stack(&lua)?;
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
    pub fn pop<V: FromLua>(&self) -> Result<V> {
        // Fast track (skip protected call)
        if !self.has_metatable() {
            return self.raw_pop();
        }

        let lua = self.0.lua.lock();
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
            V::from_stack(-1, &lua)
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
    /// table2.set_metatable(Some(always_equals_mt))?;
    ///
    /// assert!(table1.equals(&table1.clone())?);
    /// assert!(table1.equals(&table2)?);
    /// # Ok(())
    /// # }
    /// ```
    pub fn equals(&self, other: &Self) -> Result<bool> {
        if self == other {
            return Ok(true);
        }

        // Compare using `__eq` metamethod if exists
        // First, check the self for the metamethod.
        // If self does not define it, then check the other table.
        if let Some(mt) = self.metatable() {
            if mt.contains_key("__eq")? {
                return mt.get::<Function>("__eq")?.call((self, other));
            }
        }
        if let Some(mt) = other.metatable() {
            if mt.contains_key("__eq")? {
                return mt.get::<Function>("__eq")?.call((self, other));
            }
        }

        Ok(false)
    }

    /// Sets a key-value pair without invoking metamethods.
    pub fn raw_set(&self, key: impl IntoLua, value: impl IntoLua) -> Result<()> {
        let lua = self.0.lua.lock();
        let state = lua.state();
        unsafe {
            #[cfg(feature = "luau")]
            self.check_readonly_write(&lua)?;

            let _sg = StackGuard::new(state);
            check_stack(state, 5)?;

            lua.push_ref(&self.0);
            key.push_into_stack(&lua)?;
            value.push_into_stack(&lua)?;

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
    pub fn raw_get<V: FromLua>(&self, key: impl IntoLua) -> Result<V> {
        let lua = self.0.lua.lock();
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 3)?;

            lua.push_ref(&self.0);
            key.push_into_stack(&lua)?;
            ffi::lua_rawget(state, -2);

            V::from_stack(-1, &lua)
        }
    }

    /// Inserts element value at position `idx` to the table, shifting up the elements from
    /// `table[idx]`.
    ///
    /// The worst case complexity is O(n), where n is the table length.
    pub fn raw_insert(&self, idx: Integer, value: impl IntoLua) -> Result<()> {
        let size = self.raw_len() as Integer;
        if idx < 1 || idx > size + 1 {
            return Err(Error::runtime("index out of bounds"));
        }

        let lua = self.0.lua.lock();
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 5)?;

            lua.push_ref(&self.0);
            value.push_into_stack(&lua)?;
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
    pub fn raw_push(&self, value: impl IntoLua) -> Result<()> {
        let lua = self.0.lua.lock();
        let state = lua.state();
        unsafe {
            #[cfg(feature = "luau")]
            self.check_readonly_write(&lua)?;

            let _sg = StackGuard::new(state);
            check_stack(state, 4)?;

            lua.push_ref(&self.0);
            value.push_into_stack(&lua)?;

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
    pub fn raw_pop<V: FromLua>(&self) -> Result<V> {
        let lua = self.0.lua.lock();
        let state = lua.state();
        unsafe {
            #[cfg(feature = "luau")]
            self.check_readonly_write(&lua)?;

            let _sg = StackGuard::new(state);
            check_stack(state, 3)?;

            lua.push_ref(&self.0);
            let len = ffi::lua_rawlen(state, -1) as Integer;
            ffi::lua_rawgeti(state, -1, len);
            // Set slot to nil (it must be safe to do)
            ffi::lua_pushnil(state);
            ffi::lua_rawseti(state, -3, len);

            V::from_stack(-1, &lua)
        }
    }

    /// Removes a key from the table.
    ///
    /// If `key` is an integer, mlua shifts down the elements from `table[key+1]`,
    /// and erases element `table[key]`. The complexity is `O(n)` in the worst case,
    /// where `n` is the table length.
    ///
    /// For other key types this is equivalent to setting `table[key] = nil`.
    pub fn raw_remove(&self, key: impl IntoLua) -> Result<()> {
        let lua = self.0.lua.lock();
        let state = lua.state();
        let key = key.into_lua(lua.lua())?;
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
        let lua = self.0.lua.lock();
        unsafe {
            #[cfg(feature = "luau")]
            {
                self.check_readonly_write(&lua)?;
                ffi::lua_cleartable(lua.ref_thread(), self.0.index);
            }

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
    /// This might invoke the `__len` metamethod. Use the [`Table::raw_len`] method if that is not
    /// desired.
    pub fn len(&self) -> Result<Integer> {
        // Fast track (skip protected call)
        if !self.has_metatable() {
            return Ok(self.raw_len() as Integer);
        }

        let lua = self.0.lua.lock();
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
        let lua = self.0.lua.lock();
        unsafe { ffi::lua_rawlen(lua.ref_thread(), self.0.index) }
    }

    /// Returns `true` if the table is empty, without invoking metamethods.
    ///
    /// It checks both the array part and the hash part.
    pub fn is_empty(&self) -> bool {
        let lua = self.0.lua.lock();
        let ref_thread = lua.ref_thread();
        unsafe {
            ffi::lua_pushnil(ref_thread);
            if ffi::lua_next(ref_thread, self.0.index) == 0 {
                return true;
            }
            ffi::lua_pop(ref_thread, 2);
        }
        false
    }

    /// Returns a reference to the metatable of this table, or `None` if no metatable is set.
    ///
    /// Unlike the [`getmetatable`] Lua function, this method ignores the `__metatable` field.
    ///
    /// [`getmetatable`]: https://www.lua.org/manual/5.4/manual.html#pdf-getmetatable
    pub fn metatable(&self) -> Option<Table> {
        let lua = self.0.lua.lock();
        let ref_thread = lua.ref_thread();
        unsafe {
            if ffi::lua_getmetatable(ref_thread, self.0.index) == 0 {
                None
            } else {
                Some(Table(lua.pop_ref_thread()))
            }
        }
    }

    /// Sets or removes the metatable of this table.
    ///
    /// If `metatable` is `None`, the metatable is removed (if no metatable is set, this does
    /// nothing).
    pub fn set_metatable(&self, metatable: Option<Table>) -> Result<()> {
        #[cfg(feature = "luau")]
        if self.is_readonly() {
            return Err(Error::runtime("attempt to modify a readonly table"));
        }

        let lua = self.0.lua.lock();
        let ref_thread = lua.ref_thread();
        unsafe {
            if let Some(metatable) = &metatable {
                ffi::lua_pushvalue(ref_thread, metatable.0.index);
            } else {
                ffi::lua_pushnil(ref_thread);
            }
            ffi::lua_setmetatable(ref_thread, self.0.index);
        }
        Ok(())
    }

    /// Returns true if the table has metatable attached.
    #[doc(hidden)]
    #[inline]
    pub fn has_metatable(&self) -> bool {
        let lua = self.0.lua.lock();
        unsafe { !get_metatable_ptr(lua.ref_thread(), self.0.index).is_null() }
    }

    /// Sets `readonly` attribute on the table.
    #[cfg(any(feature = "luau", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub fn set_readonly(&self, enabled: bool) {
        let lua = self.0.lua.lock();
        let ref_thread = lua.ref_thread();
        unsafe {
            ffi::lua_setreadonly(ref_thread, self.0.index, enabled as _);
            if !enabled {
                // Reset "safeenv" flag
                ffi::lua_setsafeenv(ref_thread, self.0.index, 0);
            }
        }
    }

    /// Returns `readonly` attribute of the table.
    #[cfg(any(feature = "luau", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub fn is_readonly(&self) -> bool {
        let lua = self.0.lua.lock();
        let ref_thread = lua.ref_thread();
        unsafe { ffi::lua_getreadonly(ref_thread, self.0.index) != 0 }
    }

    /// Controls `safeenv` attribute on the table.
    ///
    /// This a special flag that activates some performance optimizations for environment tables.
    /// In particular, it controls:
    /// - Optimization of import resolution (cache values of constant keys).
    /// - Fast-path for built-in iteration with pairs/ipairs.
    /// - Fast-path for some built-in functions (fastcall).
    ///
    /// For `safeenv` environments, monkey patching or modifying values may not work as expected.
    #[cfg(any(feature = "luau", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub fn set_safeenv(&self, enabled: bool) {
        let lua = self.0.lua.lock();
        unsafe { ffi::lua_setsafeenv(lua.ref_thread(), self.0.index, enabled as _) };
    }

    /// Converts this table to a generic C pointer.
    ///
    /// Different tables will give different pointers.
    /// There is no way to convert the pointer back to its original value.
    ///
    /// Typically this function is used only for hashing and debug information.
    #[inline]
    pub fn to_pointer(&self) -> *const c_void {
        self.0.to_pointer()
    }

    /// Returns an iterator over the pairs of the table.
    ///
    /// This works like the Lua `pairs` function, but does not invoke the `__pairs` metamethod.
    ///
    /// The pairs are wrapped in a [`Result`], since they are lazily converted to `K` and `V` types.
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
    /// [Lua manual]: http://www.lua.org/manual/5.4/manual.html#pdf-next
    pub fn pairs<K: FromLua, V: FromLua>(&self) -> TablePairs<'_, K, V> {
        TablePairs {
            guard: self.0.lua.lock(),
            table: self,
            key: Some(Nil),
            _phantom: PhantomData,
        }
    }

    /// Iterates over the pairs of the table, invoking the given closure on each pair.
    ///
    /// This method is similar to [`Table::pairs`], but optimized for performance.
    /// It does not invoke the `__pairs` metamethod.
    pub fn for_each<K, V>(&self, mut f: impl FnMut(K, V) -> Result<()>) -> Result<()>
    where
        K: FromLua,
        V: FromLua,
    {
        let lua = self.0.lua.lock();
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 5)?;

            lua.push_ref(&self.0);
            ffi::lua_pushnil(state);
            while ffi::lua_next(state, -2) != 0 {
                let k = K::from_stack(-2, &lua)?;
                let v = V::from_stack(-1, &lua)?;
                f(k, v)?;
                // Keep key for next iteration
                ffi::lua_pop(state, 1);
            }
        }
        Ok(())
    }

    /// Returns an iterator over all values in the sequence part of the table.
    ///
    /// The iterator will yield all values `t[1]`, `t[2]` and so on, until a `nil` value is
    /// encountered. This mirrors the behavior of Lua's `ipairs` function but does not invoke
    /// any metamethods.
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
    pub fn sequence_values<V: FromLua>(&self) -> TableSequence<'_, V> {
        TableSequence {
            guard: self.0.lua.lock(),
            table: self,
            index: 1,
            _phantom: PhantomData,
        }
    }

    /// Iterates over the sequence part of the table, invoking the given closure on each value.
    #[doc(hidden)]
    pub fn for_each_value<V>(&self, mut f: impl FnMut(V) -> Result<()>) -> Result<()>
    where
        V: FromLua,
    {
        let lua = self.0.lua.lock();
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 4)?;

            lua.push_ref(&self.0);
            let len = ffi::lua_rawlen(state, -1);
            for i in 1..=len {
                ffi::lua_rawgeti(state, -1, i as _);
                f(V::from_stack(-1, &lua)?)?;
                ffi::lua_pop(state, 1);
            }
        }
        Ok(())
    }

    /// Sets element value at position `idx` without invoking metamethods.
    #[doc(hidden)]
    pub fn raw_seti(&self, idx: usize, value: impl IntoLua) -> Result<()> {
        let lua = self.0.lua.lock();
        let state = lua.state();
        unsafe {
            #[cfg(feature = "luau")]
            self.check_readonly_write(&lua)?;

            let _sg = StackGuard::new(state);
            check_stack(state, 5)?;

            lua.push_ref(&self.0);
            value.push_into_stack(&lua)?;

            let idx = idx.try_into().unwrap();
            if lua.unlikely_memory_error() {
                ffi::lua_rawseti(state, -2, idx);
            } else {
                protect_lua!(state, 2, 0, |state| ffi::lua_rawseti(state, -2, idx))?;
            }
        }
        Ok(())
    }

    #[cfg(feature = "serde")]
    pub(crate) fn is_array(&self) -> bool {
        let lua = self.0.lua.lock();
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
    fn check_readonly_write(&self, lua: &RawLua) -> Result<()> {
        if unsafe { ffi::lua_getreadonly(lua.ref_thread(), self.0.index) != 0 } {
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

        // Collect key/value pairs into a vector so we can sort them
        let mut pairs = self.pairs::<Value, Value>().flatten().collect::<Vec<_>>();
        // Sort keys
        pairs.sort_by(|(a, _), (b, _)| a.sort_cmp(b));
        let is_sequence = (pairs.iter().enumerate())
            .all(|(i, (k, _))| matches!(k, Value::Integer(n) if *n == (i + 1) as Integer));
        if pairs.is_empty() {
            return write!(fmt, "{{}}");
        }
        writeln!(fmt, "{{")?;
        if is_sequence {
            // Format as list
            for (_, value) in pairs {
                write!(fmt, "{}", " ".repeat(ident + 2))?;
                value.fmt_pretty(fmt, true, ident + 2, visited)?;
                writeln!(fmt, ",")?;
            }
        } else {
            fn is_simple_key(key: &[u8]) -> bool {
                key.iter().take(1).all(|c| c.is_ascii_alphabetic() || *c == b'_')
                    && key.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_')
            }

            for (key, value) in pairs {
                match key {
                    Value::String(key) if is_simple_key(&key.as_bytes()) => {
                        write!(fmt, "{}{}", " ".repeat(ident + 2), key.display())?;
                        write!(fmt, " = ")?;
                    }
                    _ => {
                        write!(fmt, "{}[", " ".repeat(ident + 2))?;
                        key.fmt_pretty(fmt, false, ident + 2, visited)?;
                        write!(fmt, "] = ")?;
                    }
                }
                value.fmt_pretty(fmt, true, ident + 2, visited)?;
                writeln!(fmt, ",")?;
            }
        }
        write!(fmt, "{}}}", " ".repeat(ident))
    }
}

impl fmt::Debug for Table {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        if fmt.alternate() {
            return self.fmt_pretty(fmt, 0, &mut HashSet::new());
        }
        fmt.debug_tuple("Table").field(&self.0).finish()
    }
}

impl<T> PartialEq<[T]> for Table
where
    T: IntoLua + Clone,
{
    fn eq(&self, other: &[T]) -> bool {
        let lua = self.0.lua.lock();
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
                match other.get(i).map(|v| v.clone().into_lua(lua.lua())) {
                    Some(Ok(other_val)) if val == other_val => continue,
                    _ => return false,
                }
            }
        }
        true
    }
}

impl<T> PartialEq<&[T]> for Table
where
    T: IntoLua + Clone,
{
    #[inline]
    fn eq(&self, other: &&[T]) -> bool {
        self == *other
    }
}

impl<T, const N: usize> PartialEq<[T; N]> for Table
where
    T: IntoLua + Clone,
{
    #[inline]
    fn eq(&self, other: &[T; N]) -> bool {
        self == &other[..]
    }
}

impl LuaType for Table {
    const TYPE_ID: c_int = ffi::LUA_TTABLE;
}

impl ObjectLike for Table {
    #[inline]
    fn get<V: FromLua>(&self, key: impl IntoLua) -> Result<V> {
        self.get(key)
    }

    #[inline]
    fn set(&self, key: impl IntoLua, value: impl IntoLua) -> Result<()> {
        self.set(key, value)
    }

    #[inline]
    fn call<R>(&self, args: impl IntoLuaMulti) -> Result<R>
    where
        R: FromLuaMulti,
    {
        // Convert table to a function and call via pcall that respects the `__call` metamethod.
        Function(self.0.copy()).call(args)
    }

    #[cfg(feature = "async")]
    #[inline]
    fn call_async<R>(&self, args: impl IntoLuaMulti) -> AsyncCallFuture<R>
    where
        R: FromLuaMulti,
    {
        Function(self.0.copy()).call_async(args)
    }

    #[inline]
    fn call_method<R>(&self, name: &str, args: impl IntoLuaMulti) -> Result<R>
    where
        R: FromLuaMulti,
    {
        self.call_function(name, (self, args))
    }

    #[cfg(feature = "async")]
    fn call_async_method<R>(&self, name: &str, args: impl IntoLuaMulti) -> AsyncCallFuture<R>
    where
        R: FromLuaMulti,
    {
        self.call_async_function(name, (self, args))
    }

    #[inline]
    fn call_function<R: FromLuaMulti>(&self, name: &str, args: impl IntoLuaMulti) -> Result<R> {
        match self.get(name)? {
            Value::Function(func) => func.call(args),
            val => {
                let msg = format!("attempt to call a {} value (function '{name}')", val.type_name());
                Err(Error::runtime(msg))
            }
        }
    }

    #[cfg(feature = "async")]
    #[inline]
    fn call_async_function<R>(&self, name: &str, args: impl IntoLuaMulti) -> AsyncCallFuture<R>
    where
        R: FromLuaMulti,
    {
        match self.get(name) {
            Ok(Value::Function(func)) => func.call_async(args),
            Ok(val) => {
                let msg = format!("attempt to call a {} value (function '{name}')", val.type_name());
                AsyncCallFuture::error(Error::RuntimeError(msg))
            }
            Err(err) => AsyncCallFuture::error(err),
        }
    }

    #[inline]
    fn to_string(&self) -> Result<StdString> {
        Value::Table(Table(self.0.copy())).to_string()
    }
}

/// A wrapped [`Table`] with customized serialization behavior.
#[cfg(feature = "serde")]
pub(crate) struct SerializableTable<'a> {
    table: &'a Table,
    options: crate::serde::de::Options,
    visited: Rc<RefCell<FxHashSet<*const c_void>>>,
}

#[cfg(feature = "serde")]
impl Serialize for Table {
    #[inline]
    fn serialize<S: Serializer>(&self, serializer: S) -> StdResult<S::Ok, S::Error> {
        SerializableTable::new(self, Default::default(), Default::default()).serialize(serializer)
    }
}

#[cfg(feature = "serde")]
impl<'a> SerializableTable<'a> {
    #[inline]
    pub(crate) fn new(
        table: &'a Table,
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

#[cfg(feature = "serde")]
impl Serialize for SerializableTable<'_> {
    fn serialize<S>(&self, serializer: S) -> StdResult<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use crate::serde::de::{check_value_for_skip, MapPairs, RecursionGuard};
        use crate::value::SerializableValue;

        let convert_result = |res: Result<()>, serialize_err: Option<S::Error>| match res {
            Ok(v) => Ok(v),
            Err(Error::SerializeError(_)) if serialize_err.is_some() => Err(serialize_err.unwrap()),
            Err(Error::SerializeError(msg)) => Err(serde::ser::Error::custom(msg)),
            Err(err) => Err(serde::ser::Error::custom(err.to_string())),
        };

        let options = self.options;
        let visited = &self.visited;
        let _guard = RecursionGuard::new(self.table, visited);

        // Array
        let len = self.table.raw_len();
        if len > 0
            || self.table.is_array()
            || (self.options.encode_empty_tables_as_array && self.table.is_empty())
        {
            let mut seq = serializer.serialize_seq(Some(len))?;
            let mut serialize_err = None;
            let res = self.table.for_each_value::<Value>(|value| {
                let skip = check_value_for_skip(&value, self.options, visited)
                    .map_err(|err| Error::SerializeError(err.to_string()))?;
                if skip {
                    // continue iteration
                    return Ok(());
                }
                seq.serialize_element(&SerializableValue::new(&value, options, Some(visited)))
                    .map_err(|err| {
                        serialize_err = Some(err);
                        Error::SerializeError(StdString::new())
                    })
            });
            convert_result(res, serialize_err)?;
            return seq.end();
        }

        // HashMap
        let mut map = serializer.serialize_map(None)?;
        let mut serialize_err = None;
        let mut process_pair = |key, value| {
            let skip_key = check_value_for_skip(&key, self.options, visited)
                .map_err(|err| Error::SerializeError(err.to_string()))?;
            let skip_value = check_value_for_skip(&value, self.options, visited)
                .map_err(|err| Error::SerializeError(err.to_string()))?;
            if skip_key || skip_value {
                // continue iteration
                return Ok(());
            }
            map.serialize_entry(
                &SerializableValue::new(&key, options, Some(visited)),
                &SerializableValue::new(&value, options, Some(visited)),
            )
            .map_err(|err| {
                serialize_err = Some(err);
                Error::SerializeError(StdString::new())
            })
        };

        let res = if !self.options.sort_keys {
            // Fast track
            self.table.for_each(process_pair)
        } else {
            MapPairs::new(self.table, self.options.sort_keys)
                .map_err(serde::ser::Error::custom)?
                .try_for_each(|kv| {
                    let (key, value) = kv?;
                    process_pair(key, value)
                })
        };
        convert_result(res, serialize_err)?;
        map.end()
    }
}

/// An iterator over the pairs of a Lua table.
///
/// This struct is created by the [`Table::pairs`] method.
///
/// [`Table::pairs`]: crate::Table::pairs
pub struct TablePairs<'a, K, V> {
    guard: LuaGuard,
    table: &'a Table,
    key: Option<Value>,
    _phantom: PhantomData<(K, V)>,
}

impl<K, V> Iterator for TablePairs<'_, K, V>
where
    K: FromLua,
    V: FromLua,
{
    type Item = Result<(K, V)>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(prev_key) = self.key.take() {
            let lua: &RawLua = &self.guard;
            let state = lua.state();

            let res = (|| unsafe {
                let _sg = StackGuard::new(state);
                check_stack(state, 5)?;

                lua.push_ref(&self.table.0);
                lua.push_value(&prev_key)?;

                // It must be safe to call `lua_next` unprotected as deleting a key from a table is
                // a permitted operation.
                // It fails only if the key is not found (never existed) which seems impossible scenario.
                if ffi::lua_next(state, -2) != 0 {
                    let key = lua.stack_value(-2, None);
                    Ok(Some((
                        key.clone(),
                        K::from_lua(key, lua.lua())?,
                        V::from_stack(-1, lua)?,
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
pub struct TableSequence<'a, V> {
    guard: LuaGuard,
    table: &'a Table,
    index: Integer,
    _phantom: PhantomData<V>,
}

impl<V> Iterator for TableSequence<'_, V>
where
    V: FromLua,
{
    type Item = Result<V>;

    fn next(&mut self) -> Option<Self::Item> {
        let lua: &RawLua = &self.guard;
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            if let Err(err) = check_stack(state, 1) {
                return Some(Err(err));
            }

            lua.push_ref(&self.table.0);
            match ffi::lua_rawgeti(state, -1, self.index) {
                ffi::LUA_TNIL => None,
                _ => {
                    self.index += 1;
                    Some(V::from_stack(-1, lua))
                }
            }
        }
    }
}

#[cfg(test)]
mod assertions {
    use super::*;

    #[cfg(not(feature = "send"))]
    static_assertions::assert_not_impl_any!(Table: Send);
    #[cfg(feature = "send")]
    static_assertions::assert_impl_all!(Table: Send, Sync);
}

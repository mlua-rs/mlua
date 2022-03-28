use std::marker::PhantomData;

#[cfg(feature = "serialize")]
use {
    rustc_hash::FxHashSet,
    serde::ser::{self, Serialize, SerializeMap, SerializeSeq, Serializer},
    std::{cell::RefCell, os::raw::c_void, result::Result as StdResult},
};

use crate::error::{Error, Result};
use crate::ffi;
use crate::function::Function;
use crate::types::{Integer, LuaRef};
use crate::util::{assert_stack, check_stack, StackGuard};
use crate::value::{FromLua, FromLuaMulti, Nil, ToLua, ToLuaMulti, Value};

#[cfg(feature = "async")]
use {futures_core::future::LocalBoxFuture, futures_util::future};

/// Handle to an internal Lua table.
#[derive(Clone, Debug)]
pub struct Table<'lua>(pub(crate) LuaRef<'lua>);

#[allow(clippy::len_without_is_empty)]
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
    pub fn set<K: ToLua<'lua>, V: ToLua<'lua>>(&self, key: K, value: V) -> Result<()> {
        let lua = self.0.lua;
        let key = key.to_lua(lua)?;
        let value = value.to_lua(lua)?;

        unsafe {
            let _sg = StackGuard::new(lua.state);
            check_stack(lua.state, 5)?;

            lua.push_ref(&self.0);
            lua.push_value(key)?;
            lua.push_value(value)?;
            protect_lua!(lua.state, 3, 0, fn(state) ffi::lua_settable(state, -3))
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
    pub fn get<K: ToLua<'lua>, V: FromLua<'lua>>(&self, key: K) -> Result<V> {
        let lua = self.0.lua;
        let key = key.to_lua(lua)?;

        let value = unsafe {
            let _sg = StackGuard::new(lua.state);
            check_stack(lua.state, 4)?;

            lua.push_ref(&self.0);
            lua.push_value(key)?;
            protect_lua!(lua.state, 2, 1, fn(state) ffi::lua_gettable(state, -2))?;

            lua.pop_value()
        };
        V::from_lua(value, lua)
    }

    /// Checks whether the table contains a non-nil value for `key`.
    pub fn contains_key<K: ToLua<'lua>>(&self, key: K) -> Result<bool> {
        let lua = self.0.lua;
        let key = key.to_lua(lua)?;

        unsafe {
            let _sg = StackGuard::new(lua.state);
            check_stack(lua.state, 4)?;

            lua.push_ref(&self.0);
            lua.push_value(key)?;
            protect_lua!(lua.state, 2, 1, fn(state) ffi::lua_gettable(state, -2))?;
            Ok(ffi::lua_isnil(lua.state, -1) == 0)
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
    pub fn raw_set<K: ToLua<'lua>, V: ToLua<'lua>>(&self, key: K, value: V) -> Result<()> {
        let lua = self.0.lua;
        let key = key.to_lua(lua)?;
        let value = value.to_lua(lua)?;

        unsafe {
            let _sg = StackGuard::new(lua.state);
            check_stack(lua.state, 5)?;

            lua.push_ref(&self.0);
            lua.push_value(key)?;
            lua.push_value(value)?;
            protect_lua!(lua.state, 3, 0, fn(state) ffi::lua_rawset(state, -3))
        }
    }

    /// Gets the value associated to `key` without invoking metamethods.
    pub fn raw_get<K: ToLua<'lua>, V: FromLua<'lua>>(&self, key: K) -> Result<V> {
        let lua = self.0.lua;
        let key = key.to_lua(lua)?;

        let value = unsafe {
            let _sg = StackGuard::new(lua.state);
            check_stack(lua.state, 3)?;

            lua.push_ref(&self.0);
            lua.push_value(key)?;
            ffi::lua_rawget(lua.state, -2);

            lua.pop_value()
        };
        V::from_lua(value, lua)
    }

    /// Inserts element value at position `idx` to the table, shifting up the elements from `table[idx]`.
    /// The worst case complexity is O(n), where n is the table length.
    pub fn raw_insert<V: ToLua<'lua>>(&self, idx: Integer, value: V) -> Result<()> {
        let lua = self.0.lua;
        let size = self.raw_len();
        if idx < 1 || idx > size + 1 {
            return Err(Error::RuntimeError("index out of bounds".to_string()));
        }

        let value = value.to_lua(lua)?;
        unsafe {
            let _sg = StackGuard::new(lua.state);
            check_stack(lua.state, 5)?;

            lua.push_ref(&self.0);
            lua.push_value(value)?;
            protect_lua!(lua.state, 2, 0, |state| {
                for i in (idx..=size).rev() {
                    // table[i+1] = table[i]
                    ffi::lua_rawgeti(state, -2, i);
                    ffi::lua_rawseti(state, -3, i + 1);
                }
                ffi::lua_rawseti(state, -2, idx)
            })
        }
    }

    /// Removes a key from the table.
    ///
    /// If `key` is an integer, mlua shifts down the elements from `table[key+1]`,
    /// and erases element `table[key]`. The complexity is O(n) in the worst case,
    /// where n is the table length.
    ///
    /// For other key types this is equivalent to setting `table[key] = nil`.
    pub fn raw_remove<K: ToLua<'lua>>(&self, key: K) -> Result<()> {
        let lua = self.0.lua;
        let key = key.to_lua(lua)?;
        match key {
            Value::Integer(idx) => {
                let size = self.raw_len();
                if idx < 1 || idx > size {
                    return Err(Error::RuntimeError("index out of bounds".to_string()));
                }
                unsafe {
                    let _sg = StackGuard::new(lua.state);
                    check_stack(lua.state, 4)?;

                    lua.push_ref(&self.0);
                    protect_lua!(lua.state, 1, 0, |state| {
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

    /// Returns the result of the Lua `#` operator.
    ///
    /// This might invoke the `__len` metamethod. Use the [`raw_len`] method if that is not desired.
    ///
    /// [`raw_len`]: #method.raw_len
    pub fn len(&self) -> Result<Integer> {
        let lua = self.0.lua;
        unsafe {
            let _sg = StackGuard::new(lua.state);
            check_stack(lua.state, 4)?;

            lua.push_ref(&self.0);
            protect_lua!(lua.state, 1, 0, |state| ffi::luaL_len(state, -1))
        }
    }

    /// Returns the result of the Lua `#` operator, without invoking the `__len` metamethod.
    pub fn raw_len(&self) -> Integer {
        let lua = self.0.lua;
        unsafe {
            let _sg = StackGuard::new(lua.state);
            assert_stack(lua.state, 1);

            lua.push_ref(&self.0);
            ffi::lua_rawlen(lua.state, -1) as Integer
        }
    }

    /// Returns a reference to the metatable of this table, or `None` if no metatable is set.
    ///
    /// Unlike the `getmetatable` Lua function, this method ignores the `__metatable` field.
    pub fn get_metatable(&self) -> Option<Table<'lua>> {
        let lua = self.0.lua;
        unsafe {
            let _sg = StackGuard::new(lua.state);
            assert_stack(lua.state, 2);

            lua.push_ref(&self.0);
            if ffi::lua_getmetatable(lua.state, -1) == 0 {
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
        let lua = self.0.lua;
        unsafe {
            let _sg = StackGuard::new(lua.state);
            assert_stack(lua.state, 2);

            lua.push_ref(&self.0);
            if let Some(metatable) = metatable {
                lua.push_ref(&metatable.0);
            } else {
                ffi::lua_pushnil(lua.state);
            }
            ffi::lua_setmetatable(lua.state, -2);
        }
    }

    /// Sets `readonly` attribute on the table.
    ///
    /// Requires `feature = "luau"`
    #[cfg(feature = "luau")]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub fn set_readonly(&self, enabled: bool) {
        let lua = self.0.lua;
        unsafe {
            lua.ref_thread_exec(|refthr| {
                ffi::lua_setreadonly(refthr, self.0.index, enabled as _);
                if !enabled {
                    // Reset "safeenv" flag
                    ffi::lua_setsafeenv(refthr, self.0.index, 0);
                }
            });
        }
    }

    /// Returns `readonly` attribute of the table.
    ///
    /// Requires `feature = "luau"`
    #[cfg(feature = "luau")]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub fn is_readonly(&self) -> bool {
        let lua = self.0.lua;
        unsafe { lua.ref_thread_exec(|refthr| ffi::lua_getreadonly(refthr, self.0.index) != 0) }
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
    /// The iterator will yield all values `t[1]`, `t[2]`, and so on, until a `nil` value is
    /// encountered. This mirrors the behavior of Lua's `ipairs` function and will invoke the
    /// `__index` metamethod according to the usual rules. However, the deprecated `__ipairs`
    /// metatable will not be called.
    ///
    /// Just like [`pairs`], the values are wrapped in a [`Result`].
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
            raw: false,
            _phantom: PhantomData,
        }
    }

    /// Consume this table and return an iterator over all values in the sequence part of the table.
    ///
    /// Unlike the `sequence_values`, does not invoke `__index` metamethod when iterating.
    ///
    /// [`sequence_values`]: #method.sequence_values
    pub fn raw_sequence_values<V: FromLua<'lua>>(self) -> TableSequence<'lua, V> {
        TableSequence {
            table: self.0,
            index: Some(1),
            len: None,
            raw: true,
            _phantom: PhantomData,
        }
    }

    #[cfg(any(feature = "serialize"))]
    pub(crate) fn raw_sequence_values_by_len<V: FromLua<'lua>>(
        self,
        len: Option<Integer>,
    ) -> TableSequence<'lua, V> {
        let len = len.unwrap_or_else(|| self.raw_len());
        TableSequence {
            table: self.0,
            index: Some(1),
            len: Some(len),
            raw: true,
            _phantom: PhantomData,
        }
    }

    #[cfg(feature = "serialize")]
    pub(crate) fn is_array(&self) -> bool {
        let lua = self.0.lua;
        unsafe {
            let _sg = StackGuard::new(lua.state);
            assert_stack(lua.state, 3);

            lua.push_ref(&self.0);
            if ffi::lua_getmetatable(lua.state, -1) == 0 {
                return false;
            }
            crate::serde::push_array_metatable(lua.state);
            ffi::lua_rawequal(lua.state, -1, -2) != 0
        }
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

/// An extension trait for `Table`s that provides a variety of convenient functionality.
pub trait TableExt<'lua> {
    /// Calls the table as function assuming it has `__call` metamethod.
    ///
    /// The metamethod is called with the table as its first argument, followed by the passed arguments.
    fn call<A, R>(&self, args: A) -> Result<R>
    where
        A: ToLuaMulti<'lua>,
        R: FromLuaMulti<'lua>;

    /// Asynchronously calls the table as function assuming it has `__call` metamethod.
    ///
    /// The metamethod is called with the table as its first argument, followed by the passed arguments.
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    fn call_async<'fut, A, R>(&self, args: A) -> LocalBoxFuture<'fut, Result<R>>
    where
        'lua: 'fut,
        A: ToLuaMulti<'lua>,
        R: FromLuaMulti<'lua> + 'fut;

    /// Gets the function associated to `key` from the table and executes it,
    /// passing the table itself along with `args` as function arguments.
    ///
    /// This is a shortcut for
    /// `table.get::<_, Function>(key)?.call((table.clone(), arg1, ..., argN))`
    ///
    /// This might invoke the `__index` metamethod.
    fn call_method<K, A, R>(&self, key: K, args: A) -> Result<R>
    where
        K: ToLua<'lua>,
        A: ToLuaMulti<'lua>,
        R: FromLuaMulti<'lua>;

    /// Gets the function associated to `key` from the table and executes it,
    /// passing `args` as function arguments.
    ///
    /// This is a shortcut for
    /// `table.get::<_, Function>(key)?.call(args)`
    ///
    /// This might invoke the `__index` metamethod.
    fn call_function<K, A, R>(&self, key: K, args: A) -> Result<R>
    where
        K: ToLua<'lua>,
        A: ToLuaMulti<'lua>,
        R: FromLuaMulti<'lua>;

    /// Gets the function associated to `key` from the table and asynchronously executes it,
    /// passing the table itself along with `args` as function arguments and returning Future.
    ///
    /// Requires `feature = "async"`
    ///
    /// This might invoke the `__index` metamethod.
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    fn call_async_method<'fut, K, A, R>(&self, key: K, args: A) -> LocalBoxFuture<'fut, Result<R>>
    where
        'lua: 'fut,
        K: ToLua<'lua>,
        A: ToLuaMulti<'lua>,
        R: FromLuaMulti<'lua> + 'fut;

    /// Gets the function associated to `key` from the table and asynchronously executes it,
    /// passing `args` as function arguments and returning Future.
    ///
    /// Requires `feature = "async"`
    ///
    /// This might invoke the `__index` metamethod.
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    fn call_async_function<'fut, K, A, R>(
        &self,
        key: K,
        args: A,
    ) -> LocalBoxFuture<'fut, Result<R>>
    where
        'lua: 'fut,
        K: ToLua<'lua>,
        A: ToLuaMulti<'lua>,
        R: FromLuaMulti<'lua> + 'fut;
}

impl<'lua> TableExt<'lua> for Table<'lua> {
    fn call<A, R>(&self, args: A) -> Result<R>
    where
        A: ToLuaMulti<'lua>,
        R: FromLuaMulti<'lua>,
    {
        // Convert table to a function and call via pcall that respects the `__call` metamethod.
        Function(self.0.clone()).call(args)
    }

    #[cfg(feature = "async")]
    fn call_async<'fut, A, R>(&self, args: A) -> LocalBoxFuture<'fut, Result<R>>
    where
        'lua: 'fut,
        A: ToLuaMulti<'lua>,
        R: FromLuaMulti<'lua> + 'fut,
    {
        Function(self.0.clone()).call_async(args)
    }

    fn call_method<K, A, R>(&self, key: K, args: A) -> Result<R>
    where
        K: ToLua<'lua>,
        A: ToLuaMulti<'lua>,
        R: FromLuaMulti<'lua>,
    {
        let lua = self.0.lua;
        let mut args = args.to_lua_multi(lua)?;
        args.push_front(Value::Table(self.clone()));
        self.get::<_, Function>(key)?.call(args)
    }

    fn call_function<K, A, R>(&self, key: K, args: A) -> Result<R>
    where
        K: ToLua<'lua>,
        A: ToLuaMulti<'lua>,
        R: FromLuaMulti<'lua>,
    {
        self.get::<_, Function>(key)?.call(args)
    }

    #[cfg(feature = "async")]
    fn call_async_method<'fut, K, A, R>(&self, key: K, args: A) -> LocalBoxFuture<'fut, Result<R>>
    where
        'lua: 'fut,
        K: ToLua<'lua>,
        A: ToLuaMulti<'lua>,
        R: FromLuaMulti<'lua> + 'fut,
    {
        let lua = self.0.lua;
        let mut args = match args.to_lua_multi(lua) {
            Ok(args) => args,
            Err(e) => return Box::pin(future::err(e)),
        };
        args.push_front(Value::Table(self.clone()));
        self.call_async_function(key, args)
    }

    #[cfg(feature = "async")]
    fn call_async_function<'fut, K, A, R>(&self, key: K, args: A) -> LocalBoxFuture<'fut, Result<R>>
    where
        'lua: 'fut,
        K: ToLua<'lua>,
        A: ToLuaMulti<'lua>,
        R: FromLuaMulti<'lua> + 'fut,
    {
        match self.get::<_, Function>(key) {
            Ok(func) => func.call_async(args),
            Err(e) => Box::pin(future::err(e)),
        }
    }
}

#[cfg(feature = "serialize")]
impl<'lua> Serialize for Table<'lua> {
    fn serialize<S>(&self, serializer: S) -> StdResult<S::Ok, S::Error>
    where
        S: Serializer,
    {
        thread_local! {
            static VISITED: RefCell<FxHashSet<*const c_void>> = RefCell::new(FxHashSet::default());
        }

        let lua = self.0.lua;
        let ptr = unsafe { lua.ref_thread_exec(|refthr| ffi::lua_topointer(refthr, self.0.index)) };
        let res = VISITED.with(|visited| {
            {
                let mut visited = visited.borrow_mut();
                if visited.contains(&ptr) {
                    return Err(ser::Error::custom("recursive table detected"));
                }
                visited.insert(ptr);
            }

            let len = self.raw_len() as usize;
            if len > 0 || self.is_array() {
                let mut seq = serializer.serialize_seq(Some(len))?;
                for v in self.clone().raw_sequence_values_by_len::<Value>(None) {
                    let v = v.map_err(serde::ser::Error::custom)?;
                    seq.serialize_element(&v)?;
                }
                return seq.end();
            }

            let mut map = serializer.serialize_map(None)?;
            for kv in self.clone().pairs::<Value, Value>() {
                let (k, v) = kv.map_err(serde::ser::Error::custom)?;
                map.serialize_entry(&k, &v)?;
            }
            map.end()
        });
        VISITED.with(|visited| {
            visited.borrow_mut().remove(&ptr);
        });
        res
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

            let res = (|| unsafe {
                let _sg = StackGuard::new(lua.state);
                check_stack(lua.state, 5)?;

                lua.push_ref(&self.table);
                lua.push_value(prev_key)?;

                let next = protect_lua!(lua.state, 2, ffi::LUA_MULTRET, |state| {
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
    raw: bool,
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

            let res = (|| unsafe {
                let _sg = StackGuard::new(lua.state);
                check_stack(lua.state, 1 + if self.raw { 0 } else { 3 })?;

                lua.push_ref(&self.table);
                let res = if self.raw {
                    ffi::lua_rawgeti(lua.state, -1, index)
                } else {
                    protect_lua!(lua.state, 1, 1, |state| ffi::lua_geti(state, -1, index))?
                };
                match res {
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

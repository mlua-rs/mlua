use std::borrow::Cow;
use std::collections::HashMap;
use std::ffi::CString;
use std::io::Result as IoResult;
use std::panic::Location;
use std::path::{Path, PathBuf};
use std::string::String as StdString;

use crate::error::{Error, Result};
use crate::function::Function;
use crate::state::{Lua, WeakLua};
use crate::table::Table;
use crate::traits::{FromLuaMulti, IntoLua, IntoLuaMulti};
use crate::value::Value;

/// Trait for types [loadable by Lua] and convertible to a [`Chunk`]
///
/// [loadable by Lua]: https://www.lua.org/manual/5.4/manual.html#3.3.2
pub trait AsChunk {
    /// Returns optional chunk name
    ///
    /// See [`Chunk::set_name`] for possible name prefixes.
    fn name(&self) -> Option<StdString> {
        None
    }

    /// Returns optional chunk [environment]
    ///
    /// [environment]: https://www.lua.org/manual/5.4/manual.html#2.2
    fn environment(&self, lua: &Lua) -> Result<Option<Table>> {
        let _lua = lua; // suppress warning
        Ok(None)
    }

    /// Returns optional chunk mode (text or binary)
    fn mode(&self) -> Option<ChunkMode> {
        None
    }

    /// Returns chunk data (can be text or binary)
    fn source<'a>(&self) -> IoResult<Cow<'a, [u8]>>
    where
        Self: 'a;
}

impl AsChunk for &str {
    fn source<'a>(&self) -> IoResult<Cow<'a, [u8]>>
    where
        Self: 'a,
    {
        Ok(Cow::Borrowed(self.as_bytes()))
    }
}

impl AsChunk for StdString {
    fn source<'a>(&self) -> IoResult<Cow<'a, [u8]>> {
        Ok(Cow::Owned(self.clone().into_bytes()))
    }
}

impl AsChunk for &StdString {
    fn source<'a>(&self) -> IoResult<Cow<'a, [u8]>>
    where
        Self: 'a,
    {
        Ok(Cow::Borrowed(self.as_bytes()))
    }
}

impl AsChunk for &[u8] {
    fn source<'a>(&self) -> IoResult<Cow<'a, [u8]>>
    where
        Self: 'a,
    {
        Ok(Cow::Borrowed(self))
    }
}

impl AsChunk for Vec<u8> {
    fn source<'a>(&self) -> IoResult<Cow<'a, [u8]>> {
        Ok(Cow::Owned(self.clone()))
    }
}

impl AsChunk for &Vec<u8> {
    fn source<'a>(&self) -> IoResult<Cow<'a, [u8]>>
    where
        Self: 'a,
    {
        Ok(Cow::Borrowed(self))
    }
}

impl AsChunk for &Path {
    fn name(&self) -> Option<StdString> {
        Some(format!("@{}", self.display()))
    }

    fn source<'a>(&self) -> IoResult<Cow<'a, [u8]>> {
        std::fs::read(self).map(Cow::Owned)
    }
}

impl AsChunk for PathBuf {
    fn name(&self) -> Option<StdString> {
        Some(format!("@{}", self.display()))
    }

    fn source<'a>(&self) -> IoResult<Cow<'a, [u8]>> {
        std::fs::read(self).map(Cow::Owned)
    }
}

impl<C: AsChunk + ?Sized> AsChunk for Box<C> {
    fn name(&self) -> Option<StdString> {
        (**self).name()
    }

    fn environment(&self, lua: &Lua) -> Result<Option<Table>> {
        (**self).environment(lua)
    }

    fn mode(&self) -> Option<ChunkMode> {
        (**self).mode()
    }

    fn source<'a>(&self) -> IoResult<Cow<'a, [u8]>>
    where
        Self: 'a,
    {
        (**self).source()
    }
}

/// Returned from [`Lua::load`] and is used to finalize loading and executing Lua main chunks.
#[must_use = "`Chunk`s do nothing unless one of `exec`, `eval`, `call`, or `into_function` are called on them"]
pub struct Chunk<'a> {
    pub(crate) lua: WeakLua,
    pub(crate) name: StdString,
    pub(crate) env: Result<Option<Table>>,
    pub(crate) mode: Option<ChunkMode>,
    pub(crate) source: IoResult<Cow<'a, [u8]>>,
    #[cfg(feature = "luau")]
    pub(crate) compiler: Option<Compiler>,
}

/// Represents chunk mode (text or binary).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChunkMode {
    Text,
    Binary,
}

/// Represents a constant value that can be used by Luau compiler.
#[cfg(any(feature = "luau", doc))]
#[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
#[derive(Clone, Debug)]
pub enum CompileConstant {
    Nil,
    Boolean(bool),
    Number(crate::Number),
    Vector(crate::Vector),
    String(StdString),
}

#[cfg(any(feature = "luau", doc))]
impl From<bool> for CompileConstant {
    fn from(b: bool) -> Self {
        CompileConstant::Boolean(b)
    }
}

#[cfg(any(feature = "luau", doc))]
impl From<crate::Number> for CompileConstant {
    fn from(n: crate::Number) -> Self {
        CompileConstant::Number(n)
    }
}

#[cfg(any(feature = "luau", doc))]
impl From<crate::Vector> for CompileConstant {
    fn from(v: crate::Vector) -> Self {
        CompileConstant::Vector(v)
    }
}

#[cfg(any(feature = "luau", doc))]
impl From<&str> for CompileConstant {
    fn from(s: &str) -> Self {
        CompileConstant::String(s.to_owned())
    }
}

#[cfg(any(feature = "luau", doc))]
type LibraryMemberConstantMap = HashMap<(StdString, StdString), CompileConstant>;

/// Luau compiler
#[cfg(any(feature = "luau", doc))]
#[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
#[derive(Clone, Debug)]
pub struct Compiler {
    optimization_level: u8,
    debug_level: u8,
    type_info_level: u8,
    coverage_level: u8,
    vector_lib: Option<StdString>,
    vector_ctor: Option<StdString>,
    vector_type: Option<StdString>,
    mutable_globals: Vec<StdString>,
    userdata_types: Vec<StdString>,
    libraries_with_known_members: Vec<StdString>,
    library_constants: Option<LibraryMemberConstantMap>,
    disabled_builtins: Vec<StdString>,
}

#[cfg(any(feature = "luau", doc))]
impl Default for Compiler {
    fn default() -> Self {
        const { Self::new() }
    }
}

#[cfg(any(feature = "luau", doc))]
impl Compiler {
    /// Creates Luau compiler instance with default options
    pub const fn new() -> Self {
        // Defaults are taken from luacode.h
        Compiler {
            optimization_level: 1,
            debug_level: 1,
            type_info_level: 0,
            coverage_level: 0,
            vector_lib: None,
            vector_ctor: None,
            vector_type: None,
            mutable_globals: Vec::new(),
            userdata_types: Vec::new(),
            libraries_with_known_members: Vec::new(),
            library_constants: None,
            disabled_builtins: Vec::new(),
        }
    }

    /// Sets Luau compiler optimization level.
    ///
    /// Possible values:
    /// * 0 - no optimization
    /// * 1 - baseline optimization level that doesn't prevent debuggability (default)
    /// * 2 - includes optimizations that harm debuggability such as inlining
    #[must_use]
    pub const fn set_optimization_level(mut self, level: u8) -> Self {
        self.optimization_level = level;
        self
    }

    /// Sets Luau compiler debug level.
    ///
    /// Possible values:
    /// * 0 - no debugging support
    /// * 1 - line info & function names only; sufficient for backtraces (default)
    /// * 2 - full debug info with local & upvalue names; necessary for debugger
    #[must_use]
    pub const fn set_debug_level(mut self, level: u8) -> Self {
        self.debug_level = level;
        self
    }

    /// Sets Luau type information level used to guide native code generation decisions.
    ///
    /// Possible values:
    /// * 0 - generate for native modules (default)
    /// * 1 - generate for all modules
    #[must_use]
    pub const fn set_type_info_level(mut self, level: u8) -> Self {
        self.type_info_level = level;
        self
    }

    /// Sets Luau compiler code coverage level.
    ///
    /// Possible values:
    /// * 0 - no code coverage support (default)
    /// * 1 - statement coverage
    /// * 2 - statement and expression coverage (verbose)
    #[must_use]
    pub const fn set_coverage_level(mut self, level: u8) -> Self {
        self.coverage_level = level;
        self
    }

    /// Sets alternative global builtin to construct vectors, in addition to default builtin
    /// `vector.create`.
    ///
    /// To set the library and method name, use the `lib.ctor` format.
    #[doc(hidden)]
    #[must_use]
    pub fn set_vector_ctor(mut self, ctor: impl Into<StdString>) -> Self {
        let ctor = ctor.into();
        let lib_ctor = ctor.split_once('.');
        self.vector_lib = lib_ctor.as_ref().map(|&(lib, _)| lib.to_owned());
        self.vector_ctor = (lib_ctor.as_ref())
            .map(|&(_, ctor)| ctor.to_owned())
            .or(Some(ctor));
        self
    }

    /// Sets alternative vector type name for type tables, in addition to default type `vector`.
    #[doc(hidden)]
    #[must_use]
    pub fn set_vector_type(mut self, r#type: impl Into<StdString>) -> Self {
        self.vector_type = Some(r#type.into());
        self
    }

    /// Adds a mutable global.
    ///
    /// It disables the import optimization for fields accessed through it.
    #[must_use]
    pub fn add_mutable_global(mut self, global: impl Into<StdString>) -> Self {
        self.mutable_globals.push(global.into());
        self
    }

    /// Sets a list of globals that are mutable.
    ///
    /// It disables the import optimization for fields accessed through these.
    #[must_use]
    pub fn set_mutable_globals<S: Into<StdString>>(mut self, globals: impl IntoIterator<Item = S>) -> Self {
        self.mutable_globals = globals.into_iter().map(|s| s.into()).collect();
        self
    }

    /// Adds a userdata type to the list that will be included in the type information.
    #[must_use]
    pub fn add_userdata_type(mut self, r#type: impl Into<StdString>) -> Self {
        self.userdata_types.push(r#type.into());
        self
    }

    /// Sets a list of userdata types that will be included in the type information.
    #[must_use]
    pub fn set_userdata_types<S: Into<StdString>>(mut self, types: impl IntoIterator<Item = S>) -> Self {
        self.userdata_types = types.into_iter().map(|s| s.into()).collect();
        self
    }

    /// Adds a constant for a known library member.
    ///
    /// The constants are used by the compiler to optimize the generated bytecode.
    /// Optimization level must be at least 2 for this to have any effect.
    ///
    /// The `name` is a string in the format `lib.member`, where `lib` is the library name
    /// and `member` is the member (constant) name.
    #[must_use]
    pub fn add_library_constant(
        mut self,
        name: impl AsRef<str>,
        r#const: impl Into<CompileConstant>,
    ) -> Self {
        let Some((lib, member)) = name.as_ref().split_once('.') else {
            return self;
        };
        let (lib, member) = (lib.to_owned(), member.to_owned());

        if !self.libraries_with_known_members.contains(&lib) {
            self.libraries_with_known_members.push(lib.clone());
        }
        self.library_constants
            .get_or_insert_with(HashMap::new)
            .insert((lib, member), r#const.into());
        self
    }

    /// Adds a builtin that should be disabled.
    #[must_use]
    pub fn add_disabled_builtin(mut self, builtin: impl Into<StdString>) -> Self {
        self.disabled_builtins.push(builtin.into());
        self
    }

    /// Sets a list of builtins that should be disabled.
    #[must_use]
    pub fn set_disabled_builtins<S: Into<StdString>>(
        mut self,
        builtins: impl IntoIterator<Item = S>,
    ) -> Self {
        self.disabled_builtins = builtins.into_iter().map(|s| s.into()).collect();
        self
    }

    /// Compiles the `source` into bytecode.
    ///
    /// Returns [`Error::SyntaxError`] if the source code is invalid.
    pub fn compile(&self, source: impl AsRef<[u8]>) -> Result<Vec<u8>> {
        use std::cell::RefCell;
        use std::ffi::CStr;
        use std::os::raw::{c_char, c_int};
        use std::ptr;

        let vector_lib = self.vector_lib.clone();
        let vector_lib = vector_lib.and_then(|lib| CString::new(lib).ok());
        let vector_lib = vector_lib.as_ref();
        let vector_ctor = self.vector_ctor.clone();
        let vector_ctor = vector_ctor.and_then(|ctor| CString::new(ctor).ok());
        let vector_ctor = vector_ctor.as_ref();
        let vector_type = self.vector_type.clone();
        let vector_type = vector_type.and_then(|t| CString::new(t).ok());
        let vector_type = vector_type.as_ref();

        macro_rules! vec2cstring_ptr {
            ($name:ident, $name_ptr:ident) => {
                let $name = self
                    .$name
                    .iter()
                    .map(|name| CString::new(name.clone()).ok())
                    .collect::<Option<Vec<_>>>()
                    .unwrap_or_default();
                let mut $name = $name.iter().map(|s| s.as_ptr()).collect::<Vec<_>>();
                let mut $name_ptr = ptr::null();
                if !$name.is_empty() {
                    $name.push(ptr::null());
                    $name_ptr = $name.as_ptr();
                }
            };
        }

        vec2cstring_ptr!(mutable_globals, mutable_globals_ptr);
        vec2cstring_ptr!(userdata_types, userdata_types_ptr);
        vec2cstring_ptr!(libraries_with_known_members, libraries_with_known_members_ptr);
        vec2cstring_ptr!(disabled_builtins, disabled_builtins_ptr);

        thread_local! {
            static LIBRARY_MEMBER_CONSTANT_MAP: RefCell<LibraryMemberConstantMap> = Default::default();
        }

        #[cfg(feature = "luau")]
        unsafe extern "C-unwind" fn library_member_constant_callback(
            library: *const c_char,
            member: *const c_char,
            constant: *mut ffi::lua_CompileConstant,
        ) {
            let library = CStr::from_ptr(library).to_string_lossy();
            let member = CStr::from_ptr(member).to_string_lossy();
            LIBRARY_MEMBER_CONSTANT_MAP.with_borrow(|map| {
                if let Some(cons) = map.get(&(library.to_string(), member.to_string())) {
                    match cons {
                        CompileConstant::Nil => ffi::luau_set_compile_constant_nil(constant),
                        CompileConstant::Boolean(b) => {
                            ffi::luau_set_compile_constant_boolean(constant, *b as c_int)
                        }
                        CompileConstant::Number(n) => ffi::luau_set_compile_constant_number(constant, *n),
                        CompileConstant::Vector(v) => {
                            #[cfg(not(feature = "luau-vector4"))]
                            ffi::luau_set_compile_constant_vector(constant, v.x(), v.y(), v.z(), 0.0);
                            #[cfg(feature = "luau-vector4")]
                            ffi::luau_set_compile_constant_vector(constant, v.x(), v.y(), v.z(), v.w());
                        }
                        CompileConstant::String(s) => ffi::luau_set_compile_constant_string(
                            constant,
                            s.as_ptr() as *const c_char,
                            s.len(),
                        ),
                    }
                }
            })
        }

        let bytecode = unsafe {
            let mut options = ffi::lua_CompileOptions::default();
            options.optimizationLevel = self.optimization_level as c_int;
            options.debugLevel = self.debug_level as c_int;
            options.typeInfoLevel = self.type_info_level as c_int;
            options.coverageLevel = self.coverage_level as c_int;
            options.vectorLib = vector_lib.map_or(ptr::null(), |s| s.as_ptr());
            options.vectorCtor = vector_ctor.map_or(ptr::null(), |s| s.as_ptr());
            options.vectorType = vector_type.map_or(ptr::null(), |s| s.as_ptr());
            options.mutableGlobals = mutable_globals_ptr;
            options.userdataTypes = userdata_types_ptr;
            options.librariesWithKnownMembers = libraries_with_known_members_ptr;
            if let Some(map) = self.library_constants.as_ref() {
                if !self.libraries_with_known_members.is_empty() {
                    LIBRARY_MEMBER_CONSTANT_MAP.with_borrow_mut(|gmap| *gmap = map.clone());
                    options.libraryMemberConstantCallback = Some(library_member_constant_callback);
                }
            }
            options.disabledBuiltins = disabled_builtins_ptr;
            ffi::luau_compile(source.as_ref(), options)
        };

        if bytecode.first() == Some(&0) {
            // The rest of the bytecode is the error message starting with `:`
            // See https://github.com/luau-lang/luau/blob/0.640/Compiler/src/Compiler.cpp#L4336
            let message = StdString::from_utf8_lossy(&bytecode[2..]).into_owned();
            return Err(Error::SyntaxError {
                incomplete_input: message.ends_with("<eof>"),
                message,
            });
        }

        Ok(bytecode)
    }
}

impl Chunk<'_> {
    /// Returns the name of this chunk.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Sets the name of this chunk, which results in more informative error traces.
    ///
    /// Possible name prefixes:
    /// - `@` - file path (when truncation is needed, the end of the file path is kept, as this is
    ///   more useful for identifying the file)
    /// - `=` - custom chunk name (when truncation is needed, the beginning of the name is kept)
    pub fn set_name(mut self, name: impl Into<StdString>) -> Self {
        self.name = name.into();
        self
    }

    /// Returns the environment of this chunk.
    pub fn environment(&self) -> Option<&Table> {
        self.env.as_ref().ok()?.as_ref()
    }

    /// Sets the environment of the loaded chunk to the given value.
    ///
    /// In Lua >=5.2 main chunks always have exactly one upvalue, and this upvalue is used as the
    /// `_ENV` variable inside the chunk. By default this value is set to the global environment.
    ///
    /// Calling this method changes the `_ENV` upvalue to the value provided, and variables inside
    /// the chunk will refer to the given environment rather than the global one.
    ///
    /// All global variables (including the standard library!) are looked up in `_ENV`, so it may be
    /// necessary to populate the environment in order for scripts using custom environments to be
    /// useful.
    pub fn set_environment(mut self, env: Table) -> Self {
        self.env = Ok(Some(env));
        self
    }

    /// Returns the mode (auto-detected by default) of this chunk.
    pub fn mode(&self) -> ChunkMode {
        self.detect_mode()
    }

    /// Sets whether the chunk is text or binary (autodetected by default).
    ///
    /// Be aware, Lua does not check the consistency of the code inside binary chunks.
    /// Running maliciously crafted bytecode can crash the interpreter.
    pub fn set_mode(mut self, mode: ChunkMode) -> Self {
        self.mode = Some(mode);
        self
    }

    /// Sets or overwrites a Luau compiler used for this chunk.
    ///
    /// See [`Compiler`] for details and possible options.
    #[cfg(any(feature = "luau", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub fn set_compiler(mut self, compiler: Compiler) -> Self {
        self.compiler = Some(compiler);
        self
    }

    /// Execute this chunk of code.
    ///
    /// This is equivalent to calling the chunk function with no arguments and no return values.
    pub fn exec(self) -> Result<()> {
        self.call(())
    }

    /// Asynchronously execute this chunk of code.
    ///
    /// See [`exec`] for more details.
    ///
    /// [`exec`]: Chunk::exec
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    pub async fn exec_async(self) -> Result<()> {
        self.call_async(()).await
    }

    /// Evaluate the chunk as either an expression or block.
    ///
    /// If the chunk can be parsed as an expression, this loads and executes the chunk and returns
    /// the value that it evaluates to. Otherwise, the chunk is interpreted as a block as normal,
    /// and this is equivalent to calling `exec`.
    pub fn eval<R: FromLuaMulti>(self) -> Result<R> {
        // Bytecode is always interpreted as a statement.
        // For source code, first try interpreting the lua as an expression by adding
        // "return", then as a statement. This is the same thing the
        // actual lua repl does.
        if self.detect_mode() == ChunkMode::Binary {
            self.call(())
        } else if let Ok(function) = self.to_expression() {
            function.call(())
        } else {
            self.call(())
        }
    }

    /// Asynchronously evaluate the chunk as either an expression or block.
    ///
    /// See [`eval`] for more details.
    ///
    /// [`eval`]: Chunk::eval
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    pub async fn eval_async<R>(self) -> Result<R>
    where
        R: FromLuaMulti,
    {
        if self.detect_mode() == ChunkMode::Binary {
            self.call_async(()).await
        } else if let Ok(function) = self.to_expression() {
            function.call_async(()).await
        } else {
            self.call_async(()).await
        }
    }

    /// Load the chunk function and call it with the given arguments.
    ///
    /// This is equivalent to `into_function` and calling the resulting function.
    pub fn call<R: FromLuaMulti>(self, args: impl IntoLuaMulti) -> Result<R> {
        self.into_function()?.call(args)
    }

    /// Load the chunk function and asynchronously call it with the given arguments.
    ///
    /// See [`call`] for more details.
    ///
    /// [`call`]: Chunk::call
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    pub async fn call_async<R>(self, args: impl IntoLuaMulti) -> Result<R>
    where
        R: FromLuaMulti,
    {
        self.into_function()?.call_async(args).await
    }

    /// Load this chunk into a regular [`Function`].
    ///
    /// This simply compiles the chunk without actually executing it.
    #[cfg_attr(not(feature = "luau"), allow(unused_mut))]
    pub fn into_function(mut self) -> Result<Function> {
        #[cfg(feature = "luau")]
        if self.compiler.is_some() {
            // We don't need to compile source if no compiler set
            self.compile();
        }

        let name = Self::convert_name(self.name)?;
        self.lua
            .lock()
            .load_chunk(Some(&name), self.env?.as_ref(), self.mode, self.source?.as_ref())
    }

    /// Compiles the chunk and changes mode to binary.
    ///
    /// It does nothing if the chunk is already binary or invalid.
    fn compile(&mut self) {
        if let Ok(ref source) = self.source {
            if self.detect_mode() == ChunkMode::Text {
                #[cfg(feature = "luau")]
                if let Ok(data) = self.compiler.get_or_insert_with(Default::default).compile(source) {
                    self.source = Ok(Cow::Owned(data));
                    self.mode = Some(ChunkMode::Binary);
                }
                #[cfg(not(feature = "luau"))]
                if let Ok(func) = self.lua.lock().load_chunk(None, None, None, source.as_ref()) {
                    let data = func.dump(false);
                    self.source = Ok(Cow::Owned(data));
                    self.mode = Some(ChunkMode::Binary);
                }
            }
        }
    }

    /// Fetches compiled bytecode of this chunk from the cache.
    ///
    /// If not found, compiles the source code and stores it on the cache.
    pub(crate) fn try_cache(mut self) -> Self {
        struct ChunksCache(HashMap<Vec<u8>, Vec<u8>>);

        // Try to fetch compiled chunk from cache
        let mut text_source = None;
        if let Ok(ref source) = self.source {
            if self.detect_mode() == ChunkMode::Text {
                let lua = self.lua.lock();
                if let Some(cache) = lua.priv_app_data_ref::<ChunksCache>() {
                    if let Some(data) = cache.0.get(source.as_ref()) {
                        self.source = Ok(Cow::Owned(data.clone()));
                        self.mode = Some(ChunkMode::Binary);
                        return self;
                    }
                }
                text_source = Some(source.as_ref().to_vec());
            }
        }

        // Compile and cache the chunk
        if let Some(text_source) = text_source {
            self.compile();
            if let Ok(ref binary_source) = self.source {
                if self.detect_mode() == ChunkMode::Binary {
                    let lua = self.lua.lock();
                    if let Some(mut cache) = lua.priv_app_data_mut::<ChunksCache>() {
                        cache.0.insert(text_source, binary_source.to_vec());
                    } else {
                        let mut cache = ChunksCache(HashMap::new());
                        cache.0.insert(text_source, binary_source.to_vec());
                        lua.set_priv_app_data(cache);
                    };
                }
            }
        }

        self
    }

    fn to_expression(&self) -> Result<Function> {
        // We assume that mode is Text
        let source = self.source.as_ref();
        let source = source.map_err(Error::runtime)?;
        let source = Self::expression_source(source);
        // We don't need to compile source if no compiler options set
        #[cfg(feature = "luau")]
        let source = self
            .compiler
            .as_ref()
            .map(|c| c.compile(&source))
            .transpose()?
            .unwrap_or(source);

        let name = Self::convert_name(self.name.clone())?;
        let env = match &self.env {
            Ok(Some(env)) => Some(env),
            Ok(None) => None,
            Err(err) => return Err(err.clone()),
        };
        self.lua.lock().load_chunk(Some(&name), env, None, &source)
    }

    fn detect_mode(&self) -> ChunkMode {
        if let Some(mode) = self.mode {
            return mode;
        }
        if let Ok(source) = &self.source {
            #[cfg(not(feature = "luau"))]
            if source.starts_with(ffi::LUA_SIGNATURE) {
                return ChunkMode::Binary;
            }
            #[cfg(feature = "luau")]
            if *source.first().unwrap_or(&u8::MAX) < b'\n' {
                return ChunkMode::Binary;
            }
        }
        ChunkMode::Text
    }

    fn convert_name(name: StdString) -> Result<CString> {
        CString::new(name).map_err(|err| Error::runtime(format!("invalid name: {err}")))
    }

    fn expression_source(source: &[u8]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(b"return ".len() + source.len());
        buf.extend(b"return ");
        buf.extend(source);
        buf
    }
}

struct WrappedChunk<T: AsChunk> {
    chunk: T,
    caller: &'static Location<'static>,
}

impl Chunk<'_> {
    /// Wraps a chunk of Lua code, returning an opaque type that implements [`IntoLua`] trait.
    ///
    /// The resulted `IntoLua` implementation will convert the chunk into a Lua function without
    /// executing it.
    #[doc(hidden)]
    #[track_caller]
    pub fn wrap(chunk: impl AsChunk) -> impl IntoLua {
        WrappedChunk {
            chunk,
            caller: Location::caller(),
        }
    }
}

impl<T: AsChunk> IntoLua for WrappedChunk<T> {
    fn into_lua(self, lua: &Lua) -> Result<Value> {
        lua.load_with_location(self.chunk, self.caller)
            .into_function()
            .map(Value::Function)
    }
}

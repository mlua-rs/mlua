use std::borrow::Cow;
use std::collections::HashMap;
use std::ffi::CString;
use std::io::Result as IoResult;
use std::path::{Path, PathBuf};
use std::string::String as StdString;

use crate::error::{Error, Result};
use crate::ffi;
use crate::function::Function;
use crate::lua::Lua;
use crate::value::{FromLuaMulti, ToLua, ToLuaMulti, Value};

#[cfg(feature = "async")]
use {futures_core::future::LocalBoxFuture, futures_util::future};

/// Trait for types [loadable by Lua] and convertible to a [`Chunk`]
///
/// [loadable by Lua]: https://www.lua.org/manual/5.4/manual.html#3.3.2
/// [`Chunk`]: crate::Chunk
pub trait AsChunk<'lua> {
    /// Returns chunk data (can be text or binary)
    fn source(&self) -> IoResult<Cow<[u8]>>;

    /// Returns optional chunk name
    fn name(&self) -> Option<StdString> {
        None
    }

    /// Returns optional chunk [environment]
    ///
    /// [environment]: https://www.lua.org/manual/5.4/manual.html#2.2
    fn env(&self, _lua: &'lua Lua) -> Result<Option<Value<'lua>>> {
        Ok(None)
    }

    /// Returns optional chunk mode (text or binary)
    fn mode(&self) -> Option<ChunkMode> {
        None
    }
}

impl<'lua> AsChunk<'lua> for str {
    fn source(&self) -> IoResult<Cow<[u8]>> {
        Ok(Cow::Borrowed(self.as_ref()))
    }
}

impl<'lua> AsChunk<'lua> for StdString {
    fn source(&self) -> IoResult<Cow<[u8]>> {
        Ok(Cow::Borrowed(self.as_ref()))
    }
}

impl<'lua> AsChunk<'lua> for [u8] {
    fn source(&self) -> IoResult<Cow<[u8]>> {
        Ok(Cow::Borrowed(self))
    }
}

impl<'lua> AsChunk<'lua> for Vec<u8> {
    fn source(&self) -> IoResult<Cow<[u8]>> {
        Ok(Cow::Borrowed(self))
    }
}

impl<'lua> AsChunk<'lua> for Path {
    fn source(&self) -> IoResult<Cow<[u8]>> {
        std::fs::read(self).map(Cow::Owned)
    }

    fn name(&self) -> Option<StdString> {
        Some(format!("@{}", self.display()))
    }
}

impl<'lua> AsChunk<'lua> for PathBuf {
    fn source(&self) -> IoResult<Cow<[u8]>> {
        std::fs::read(self).map(Cow::Owned)
    }

    fn name(&self) -> Option<StdString> {
        Some(format!("@{}", self.display()))
    }
}

/// Returned from [`Lua::load`] and is used to finalize loading and executing Lua main chunks.
///
/// [`Lua::load`]: crate::Lua::load
#[must_use = "`Chunk`s do nothing unless one of `exec`, `eval`, `call`, or `into_function` are called on them"]
pub struct Chunk<'lua, 'a> {
    pub(crate) lua: &'lua Lua,
    pub(crate) source: IoResult<Cow<'a, [u8]>>,
    pub(crate) name: Option<StdString>,
    pub(crate) env: Result<Option<Value<'lua>>>,
    pub(crate) mode: Option<ChunkMode>,
    #[cfg(feature = "luau")]
    pub(crate) compiler: Option<Compiler>,
}

/// Represents chunk mode (text or binary).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChunkMode {
    Text,
    Binary,
}

/// Luau compiler
#[cfg(any(feature = "luau", doc))]
#[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
#[derive(Clone, Debug)]
pub struct Compiler {
    optimization_level: u8,
    debug_level: u8,
    coverage_level: u8,
    vector_lib: Option<String>,
    vector_ctor: Option<String>,
    mutable_globals: Vec<String>,
}

#[cfg(any(feature = "luau", doc))]
impl Default for Compiler {
    fn default() -> Self {
        // Defaults are taken from luacode.h
        Compiler {
            optimization_level: 1,
            debug_level: 1,
            coverage_level: 0,
            vector_lib: None,
            vector_ctor: None,
            mutable_globals: Vec::new(),
        }
    }
}

#[cfg(any(feature = "luau", doc))]
impl Compiler {
    /// Creates Luau compiler instance with default options
    pub fn new() -> Self {
        Compiler::default()
    }

    /// Sets Luau compiler optimization level.
    ///
    /// Possible values:
    /// * 0 - no optimization
    /// * 1 - baseline optimization level that doesn't prevent debuggability (default)
    /// * 2 - includes optimizations that harm debuggability such as inlining
    pub fn set_optimization_level(mut self, level: u8) -> Self {
        self.optimization_level = level;
        self
    }

    /// Sets Luau compiler debug level.
    ///
    /// Possible values:
    /// * 0 - no debugging support
    /// * 1 - line info & function names only; sufficient for backtraces (default)
    /// * 2 - full debug info with local & upvalue names; necessary for debugger
    pub fn set_debug_level(mut self, level: u8) -> Self {
        self.debug_level = level;
        self
    }

    /// Sets Luau compiler code coverage level.
    ///
    /// Possible values:
    /// * 0 - no code coverage support (default)
    /// * 1 - statement coverage
    /// * 2 - statement and expression coverage (verbose)
    pub fn set_coverage_level(mut self, level: u8) -> Self {
        self.coverage_level = level;
        self
    }

    #[doc(hidden)]
    pub fn set_vector_lib(mut self, lib: Option<String>) -> Self {
        self.vector_lib = lib;
        self
    }

    #[doc(hidden)]
    pub fn set_vector_ctor(mut self, ctor: Option<String>) -> Self {
        self.vector_ctor = ctor;
        self
    }

    /// Sets a list of globals that are mutable.
    ///
    /// It disables the import optimization for fields accessed through these.
    pub fn set_mutable_globals(mut self, globals: Vec<String>) -> Self {
        self.mutable_globals = globals;
        self
    }

    /// Compiles the `source` into bytecode.
    pub fn compile(&self, source: impl AsRef<[u8]>) -> Vec<u8> {
        use std::os::raw::c_int;
        use std::ptr;

        let vector_lib = self.vector_lib.clone();
        let vector_lib = vector_lib.and_then(|lib| CString::new(lib).ok());
        let vector_lib = vector_lib.as_ref();
        let vector_ctor = self.vector_ctor.clone();
        let vector_ctor = vector_ctor.and_then(|ctor| CString::new(ctor).ok());
        let vector_ctor = vector_ctor.as_ref();

        let mutable_globals = self
            .mutable_globals
            .iter()
            .map(|name| CString::new(name.clone()).ok())
            .collect::<Option<Vec<_>>>()
            .unwrap_or_default();
        let mut mutable_globals = mutable_globals
            .iter()
            .map(|s| s.as_ptr())
            .collect::<Vec<_>>();
        let mut mutable_globals_ptr = ptr::null_mut();
        if !mutable_globals.is_empty() {
            mutable_globals.push(ptr::null());
            mutable_globals_ptr = mutable_globals.as_mut_ptr();
        }

        unsafe {
            let options = ffi::lua_CompileOptions {
                optimizationLevel: self.optimization_level as c_int,
                debugLevel: self.debug_level as c_int,
                coverageLevel: self.coverage_level as c_int,
                vectorLib: vector_lib.map_or(ptr::null(), |s| s.as_ptr()),
                vectorCtor: vector_ctor.map_or(ptr::null(), |s| s.as_ptr()),
                mutableGlobals: mutable_globals_ptr,
            };
            ffi::luau_compile(source.as_ref(), options)
        }
    }
}

impl<'lua, 'a> Chunk<'lua, 'a> {
    /// Sets the name of this chunk, which results in more informative error traces.
    pub fn set_name(mut self, name: impl AsRef<str>) -> Result<Self> {
        self.name = Some(name.as_ref().to_string());
        // Do extra validation
        let _ = self.convert_name()?;
        Ok(self)
    }

    /// Sets the first upvalue (`_ENV`) of the loaded chunk to the given value.
    ///
    /// Lua main chunks always have exactly one upvalue, and this upvalue is used as the `_ENV`
    /// variable inside the chunk. By default this value is set to the global environment.
    ///
    /// Calling this method changes the `_ENV` upvalue to the value provided, and variables inside
    /// the chunk will refer to the given environment rather than the global one.
    ///
    /// All global variables (including the standard library!) are looked up in `_ENV`, so it may be
    /// necessary to populate the environment in order for scripts using custom environments to be
    /// useful.
    pub fn set_environment<V: ToLua<'lua>>(mut self, env: V) -> Result<Self> {
        // Prefer to propagate errors here and wrap to `Ok`
        self.env = Ok(Some(env.to_lua(self.lua)?));
        Ok(self)
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
    ///
    /// Requires `feature = "luau"`
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
        self.call(())?;
        Ok(())
    }

    /// Asynchronously execute this chunk of code.
    ///
    /// See [`exec`] for more details.
    ///
    /// Requires `feature = "async"`
    ///
    /// [`exec`]: #method.exec
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    pub fn exec_async<'fut>(self) -> LocalBoxFuture<'fut, Result<()>>
    where
        'lua: 'fut,
    {
        self.call_async(())
    }

    /// Evaluate the chunk as either an expression or block.
    ///
    /// If the chunk can be parsed as an expression, this loads and executes the chunk and returns
    /// the value that it evaluates to. Otherwise, the chunk is interpreted as a block as normal,
    /// and this is equivalent to calling `exec`.
    pub fn eval<R: FromLuaMulti<'lua>>(self) -> Result<R> {
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
    /// Requires `feature = "async"`
    ///
    /// [`eval`]: #method.eval
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    pub fn eval_async<'fut, R>(self) -> LocalBoxFuture<'fut, Result<R>>
    where
        'lua: 'fut,
        R: FromLuaMulti<'lua> + 'fut,
    {
        if self.detect_mode() == ChunkMode::Binary {
            self.call_async(())
        } else if let Ok(function) = self.to_expression() {
            function.call_async(())
        } else {
            self.call_async(())
        }
    }

    /// Load the chunk function and call it with the given arguments.
    ///
    /// This is equivalent to `into_function` and calling the resulting function.
    pub fn call<A: ToLuaMulti<'lua>, R: FromLuaMulti<'lua>>(self, args: A) -> Result<R> {
        self.into_function()?.call(args)
    }

    /// Load the chunk function and asynchronously call it with the given arguments.
    ///
    /// See [`call`] for more details.
    ///
    /// Requires `feature = "async"`
    ///
    /// [`call`]: #method.call
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    pub fn call_async<'fut, A, R>(self, args: A) -> LocalBoxFuture<'fut, Result<R>>
    where
        'lua: 'fut,
        A: ToLuaMulti<'lua>,
        R: FromLuaMulti<'lua> + 'fut,
    {
        match self.into_function() {
            Ok(func) => func.call_async(args),
            Err(e) => Box::pin(future::err(e)),
        }
    }

    /// Load this chunk into a regular `Function`.
    ///
    /// This simply compiles the chunk without actually executing it.
    #[cfg_attr(not(feature = "luau"), allow(unused_mut))]
    pub fn into_function(mut self) -> Result<Function<'lua>> {
        #[cfg(feature = "luau")]
        if self.compiler.is_some() {
            // We don't need to compile source if no compiler set
            self.compile();
        }

        let name = self.convert_name()?;
        self.lua
            .load_chunk(self.source?.as_ref(), name.as_deref(), self.env?, self.mode)
    }

    /// Compiles the chunk and changes mode to binary.
    ///
    /// It does nothing if the chunk is already binary.
    fn compile(&mut self) {
        if let Ok(ref source) = self.source {
            if self.detect_mode() == ChunkMode::Text {
                #[cfg(feature = "luau")]
                {
                    let data = self
                        .compiler
                        .get_or_insert_with(Default::default)
                        .compile(source);
                    self.source = Ok(Cow::Owned(data));
                    self.mode = Some(ChunkMode::Binary);
                }
                #[cfg(not(feature = "luau"))]
                if let Ok(func) = self.lua.load_chunk(source.as_ref(), None, None, None) {
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
                if let Some(cache) = self.lua.app_data_ref::<ChunksCache>() {
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
                    if let Some(mut cache) = self.lua.app_data_mut::<ChunksCache>() {
                        cache.0.insert(text_source, binary_source.as_ref().to_vec());
                    } else {
                        let mut cache = ChunksCache(HashMap::new());
                        cache.0.insert(text_source, binary_source.as_ref().to_vec());
                        self.lua.set_app_data(cache);
                    }
                }
            }
        }

        self
    }

    fn to_expression(&self) -> Result<Function<'lua>> {
        // We assume that mode is Text
        let source = self.source.as_ref();
        let source = source.map_err(|err| Error::RuntimeError(err.to_string()))?;
        let source = Self::expression_source(source);
        // We don't need to compile source if no compiler options set
        #[cfg(feature = "luau")]
        let source = self
            .compiler
            .as_ref()
            .map(|c| c.compile(&source))
            .unwrap_or(source);

        let name = self.convert_name()?;
        self.lua
            .load_chunk(&source, name.as_deref(), self.env.clone()?, None)
    }

    fn detect_mode(&self) -> ChunkMode {
        match (self.mode, &self.source) {
            (Some(mode), _) => mode,
            (None, Ok(source)) => {
                #[cfg(not(feature = "luau"))]
                if source.starts_with(ffi::LUA_SIGNATURE) {
                    return ChunkMode::Binary;
                }
                #[cfg(feature = "luau")]
                if *source.first().unwrap_or(&u8::MAX) < b'\n' {
                    return ChunkMode::Binary;
                }
                ChunkMode::Text
            }
            (None, Err(_)) => ChunkMode::Text, // any value is fine
        }
    }

    fn convert_name(&self) -> Result<Option<CString>> {
        self.name
            .clone()
            .map(CString::new)
            .transpose()
            .map_err(|err| Error::RuntimeError(format!("invalid name: {err}")))
    }

    fn expression_source(source: &[u8]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(b"return ".len() + source.len());
        buf.extend(b"return ");
        buf.extend(source);
        buf
    }
}

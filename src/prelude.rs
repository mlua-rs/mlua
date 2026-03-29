//! Re-exports most types with an extra `Lua*` prefix to prevent name clashes.

#[doc(no_inline)]
pub use crate::{
    AnyUserData as LuaAnyUserData, BorrowedBytes as LuaBorrowedBytes, BorrowedStr as LuaBorrowedStr,
    Chunk as LuaChunk, ChunkMode as LuaChunkMode, Either as LuaEither, Error as LuaError, FromLua,
    FromLuaMulti, Function as LuaFunction, Integer as LuaInteger, IntoLua, IntoLuaMulti,
    LightUserData as LuaLightUserData, Lua, LuaNativeFn, LuaNativeFnMut, LuaOptions, LuaString,
    MetaMethod as LuaMetaMethod, MultiValue as LuaMultiValue, Nil as LuaNil, Number as LuaNumber,
    ObjectLike as LuaObjectLike, RegistryKey as LuaRegistryKey, Result as LuaResult, StdLib as LuaStdLib,
    Table as LuaTable, Thread as LuaThread, UserData as LuaUserData, UserDataFields as LuaUserDataFields,
    UserDataMetatable as LuaUserDataMetatable, UserDataMethods as LuaUserDataMethods,
    UserDataRef as LuaUserDataRef, UserDataRefMut as LuaUserDataRefMut,
    UserDataRegistry as LuaUserDataRegistry, Value as LuaValue, Variadic as LuaVariadic,
    VmState as LuaVmState, WeakLua, error::ErrorContext as LuaErrorContext,
    error::ExternalError as LuaExternalError, error::ExternalResult as LuaExternalResult,
    function::FunctionInfo as LuaFunctionInfo, state::GcIncParams as LuaGcIncParams,
    state::GcMode as LuaGcMode, table::TablePairs as LuaTablePairs, table::TableSequence as LuaTableSequence,
    thread::ThreadStatus as LuaThreadStatus,
};

#[cfg(not(feature = "luau"))]
#[doc(no_inline)]
pub use crate::HookTriggers as LuaHookTriggers;

#[cfg(any(feature = "lua54", feature = "lua55"))]
#[doc(no_inline)]
pub use crate::state::GcGenParams as LuaGcGenParams;

#[cfg(feature = "luau")]
#[doc(no_inline)]
pub use crate::{
    CompileConstant as LuaCompileConstant, Compiler as LuaCompiler, Vector as LuaVector,
    luau::{
        FsRequirer as LuaFsRequirer, HeapDump as LuaHeapDump, NavigateError as LuaNavigateError,
        Require as LuaRequire,
    },
};

#[cfg(feature = "async")]
#[doc(no_inline)]
pub use crate::{LuaNativeAsyncFn, thread::AsyncThread as LuaAsyncThread};

#[cfg(feature = "serde")]
#[doc(no_inline)]
pub use crate::{
    DeserializeOptions as LuaDeserializeOptions, LuaSerdeExt, SerializableValue as LuaSerializableValue,
    SerializeOptions as LuaSerializeOptions,
};

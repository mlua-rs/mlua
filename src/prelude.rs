//! Re-exports most types with an extra `Lua*` prefix to prevent name clashes.

#[doc(no_inline)]
pub use crate::{
    AnyUserData as LuaAnyUserData, BorrowedBytes as LuaBorrowedBytes, BorrowedStr as LuaBorrowedStr,
    Chunk as LuaChunk, Either as LuaEither, Error as LuaError, ErrorContext as LuaErrorContext,
    ExternalError as LuaExternalError, ExternalResult as LuaExternalResult, FromLua, FromLuaMulti,
    Function as LuaFunction, GCMode as LuaGCMode, Integer as LuaInteger, IntoLua, IntoLuaMulti,
    LightUserData as LuaLightUserData, Lua, LuaNativeFn, LuaNativeFnMut, LuaOptions, LuaString,
    MetaMethod as LuaMetaMethod, MultiValue as LuaMultiValue, Nil as LuaNil, Number as LuaNumber,
    ObjectLike as LuaObjectLike, RegistryKey as LuaRegistryKey, Result as LuaResult, StdLib as LuaStdLib,
    Table as LuaTable, Thread as LuaThread, ThreadStatus as LuaThreadStatus, UserData as LuaUserData,
    UserDataFields as LuaUserDataFields, UserDataMetatable as LuaUserDataMetatable,
    UserDataMethods as LuaUserDataMethods, UserDataRef as LuaUserDataRef,
    UserDataRefMut as LuaUserDataRefMut, UserDataRegistry as LuaUserDataRegistry, Value as LuaValue,
    Variadic as LuaVariadic, VmState as LuaVmState, WeakLua, function::FunctionInfo as LuaFunctionInfo,
    table::TablePairs as LuaTablePairs, table::TableSequence as LuaTableSequence,
};

#[cfg(not(feature = "luau"))]
#[doc(no_inline)]
pub use crate::HookTriggers as LuaHookTriggers;

#[cfg(feature = "luau")]
#[doc(no_inline)]
pub use crate::{
    CompileConstant as LuaCompileConstant, NavigateError as LuaNavigateError, Require as LuaRequire,
    TextRequirer as LuaTextRequirer, Vector as LuaVector,
};

#[cfg(feature = "async")]
#[doc(no_inline)]
pub use crate::{AsyncThread as LuaAsyncThread, LuaNativeAsyncFn};

#[cfg(feature = "serde")]
#[doc(no_inline)]
pub use crate::{
    DeserializeOptions as LuaDeserializeOptions, LuaSerdeExt, SerializableValue as LuaSerializableValue,
    SerializeOptions as LuaSerializeOptions,
};

//! Re-exports most types with an extra `Lua*` prefix to prevent name clashes.

#[doc(no_inline)]
pub use crate::{
    AnyUserData as LuaAnyUserData, AnyUserDataExt as LuaAnyUserDataExt, Chunk as LuaChunk,
    Error as LuaError, ErrorContext as LuaErrorContext, FromLua, FromLuaMulti,
    Function as LuaFunction, FunctionInfo as LuaFunctionInfo, GCMode as LuaGCMode,
    Integer as LuaInteger, IntoLua, IntoLuaMulti, LightUserData as LuaLightUserData, Lua,
    LuaOptions, MetaMethod as LuaMetaMethod, MultiValue as LuaMultiValue, Nil as LuaNil,
    Number as LuaNumber, RegistryKey as LuaRegistryKey, Result as LuaResult, StdLib as LuaStdLib,
    String as LuaString, Table as LuaTable, TableExt as LuaTableExt, TablePairs as LuaTablePairs,
    TableSequence as LuaTableSequence, Thread as LuaThread, ThreadStatus as LuaThreadStatus,
    UserData as LuaUserData, UserDataFields as LuaUserDataFields,
    UserDataMetatable as LuaUserDataMetatable, UserDataMethods as LuaUserDataMethods,
    UserDataRef as LuaUserDataRef, UserDataRefMut as LuaUserDataRefMut,
    UserDataRegistry as LuaUserDataRegistry, Value as LuaValue,
};

#[cfg(feature = "std")]
pub use crate::{ExternalError as LuaExternalError, ExternalResult as LuaExternalResult};

#[cfg(not(feature = "luau"))]
#[doc(no_inline)]
pub use crate::HookTriggers as LuaHookTriggers;

#[cfg(feature = "luau")]
#[doc(no_inline)]
pub use crate::{CoverageInfo as LuaCoverageInfo, Vector as LuaVector, VmState as LuaVmState};

#[cfg(feature = "async")]
#[doc(no_inline)]
pub use crate::AsyncThread as LuaAsyncThread;

#[cfg(feature = "serialize")]
#[doc(no_inline)]
pub use crate::{
    DeserializeOptions as LuaDeserializeOptions, LuaSerdeExt,
    SerializeOptions as LuaSerializeOptions,
};

#[cfg(feature = "unstable")]
#[doc(no_inline)]
pub use crate::{
    OwnedAnyUserData as LuaOwnedAnyUserData, OwnedFunction as LuaOwnedFunction,
    OwnedString as LuaOwnedString, OwnedTable as LuaOwnedTable, OwnedThread as LuaOwnedThread,
};

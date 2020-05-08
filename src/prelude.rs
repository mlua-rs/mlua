//! Re-exports most types with an extra `Lua*` prefix to prevent name clashes.

pub use crate::{
    AnyUserData as LuaAnyUserData, Chunk as LuaChunk, Error as LuaError,
    ExternalError as LuaExternalError, ExternalResult as LuaExternalResult, FromLua, FromLuaMulti,
    Function as LuaFunction, GCMode as LuaGCMode, Integer as LuaInteger,
    LightUserData as LuaLightUserData, Lua, MetaMethod as LuaMetaMethod,
    MultiValue as LuaMultiValue, Nil as LuaNil, Number as LuaNumber, RegistryKey as LuaRegistryKey,
    Result as LuaResult, String as LuaString, Table as LuaTable, TableExt as LuaTableExt,
    TablePairs as LuaTablePairs, TableSequence as LuaTableSequence, Thread as LuaThread,
    ThreadStatus as LuaThreadStatus, ToLua, ToLuaMulti, UserData as LuaUserData,
    UserDataMethods as LuaUserDataMethods, Value as LuaValue,
};

#[cfg(feature = "async")]
pub use crate::AsyncThread as LuaAsyncThread;

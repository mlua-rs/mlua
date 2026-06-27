#![cfg(feature = "luau")]

use std::fmt::Debug;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use mlua::{
    Compiler, Error, Function, Lua, LuaOptions, ObjectLike, Result, StdLib, Table, Value, Vector, VmState,
};

#[test]
fn test_version() -> Result<()> {
    let lua = Lua::new();
    assert!(lua.globals().get::<String>("_VERSION")?.starts_with("Luau 0."));
    Ok(())
}

#[cfg(not(feature = "luau-vector4"))]
#[test]
fn test_vectors() -> Result<()> {
    let lua = Lua::new();

    let v: Value = lua
        .load("vector.create(1, 2, 3) + vector.create(3, 2, 1)")
        .eval()?;
    assert!(v.is_vector());
    assert_eq!(v.as_vector().unwrap(), [4.0, 4.0, 4.0]);

    // Test conversion into Rust array
    let v: [f64; 3] = lua.load("vector.create(1, 2, 3)").eval()?;
    assert!(v == [1.0, 2.0, 3.0]);

    // Test vector methods
    lua.load(
        r#"
        local v = vector.create(1, 2, 3)
        assert(v.x == 1)
        assert(v.y == 2)
        assert(v.z == 3)
    "#,
    )
    .exec()?;

    // Test vector methods (fastcall)
    lua.load(
        r#"
        local v = vector.create(1, 2, 3)
        assert(v.x == 1)
        assert(v.y == 2)
        assert(v.z == 3)
    "#,
    )
    .set_compiler(Compiler::new().set_vector_ctor("vector"))
    .exec()?;

    Ok(())
}

#[cfg(feature = "luau-vector4")]
#[test]
fn test_vectors() -> Result<()> {
    let lua = Lua::new();

    let v: Value = lua
        .load("vector.create(1, 2, 3, 4) + vector.create(4, 3, 2, 1)")
        .eval()?;
    assert!(v.is_vector());
    assert_eq!(v.as_vector().unwrap(), [5.0, 5.0, 5.0, 5.0]);

    // Test conversion into Rust array
    let v: [f64; 4] = lua.load("vector.create(1, 2, 3, 4)").eval()?;
    assert!(v == [1.0, 2.0, 3.0, 4.0]);

    // Test vector methods
    lua.load(
        r#"
        local v = vector.create(1, 2, 3, 4)
        assert(v.x == 1)
        assert(v.y == 2)
        assert(v.z == 3)
        assert(v.w == 4)
    "#,
    )
    .exec()?;

    // Test vector methods (fastcall)
    lua.load(
        r#"
        local v = vector.create(1, 2, 3, 4)
        assert(v.x == 1)
        assert(v.y == 2)
        assert(v.z == 3)
        assert(v.w == 4)
    "#,
    )
    .set_compiler(Compiler::new().set_vector_ctor("vector"))
    .exec()?;

    Ok(())
}

#[test]
fn test_vector_metatable() -> Result<()> {
    let lua = Lua::new();

    let vector_mt = lua
        .load(
            r#"
            {
                __index = {
                    new = vector.create,

                    product = function(a, b)
                        return vector.create(a.x * b.x, a.y * b.y, a.z * b.z)
                    end
                }
            }
    "#,
        )
        .eval::<Table>()?;
    vector_mt.set_metatable(Some(vector_mt.clone()))?;
    lua.set_type_metatable::<Vector>(Some(vector_mt.clone()));
    lua.globals().set("Vector3", vector_mt)?;

    let compiler = Compiler::new()
        .set_vector_ctor("Vector3.new")
        .set_vector_type("Vector3");

    // Test vector methods (fastcall)
    lua.load(
        r#"
        local v = Vector3.new(1, 2, 3)
        local v2 = v:product(Vector3.new(2, 3, 4))
        assert(v2.x == 2 and v2.y == 6 and v2.z == 12)
    "#,
    )
    .set_compiler(compiler)
    .exec()?;

    Ok(())
}

#[test]
fn test_readonly_table() -> Result<()> {
    let lua = Lua::new();

    let t = lua.create_sequence_from([1])?;
    assert!(!t.is_readonly());
    t.set_readonly(true);
    assert!(t.is_readonly());

    #[track_caller]
    fn check_readonly_error<T: Debug>(res: Result<T>) {
        match res {
            Err(Error::RuntimeError(e)) if e.contains("attempt to modify a readonly table") => {}
            r => panic!("expected RuntimeError(...) with a specific message, got {r:?}"),
        }
    }

    check_readonly_error(t.set("key", "value"));
    check_readonly_error(t.raw_set("key", "value"));
    check_readonly_error(t.raw_insert(1, "value"));
    check_readonly_error(t.raw_remove(1));
    check_readonly_error(t.push("value"));
    check_readonly_error(t.pop::<Value>());
    check_readonly_error(t.raw_push("value"));
    check_readonly_error(t.raw_pop::<Value>());

    // Special case
    match t.set_metatable(None) {
        Err(Error::RuntimeError(e)) if e.contains("attempt to modify a readonly table") => {}
        r => panic!("expected RuntimeError(...) with a specific message, got {r:?}"),
    }

    Ok(())
}

#[test]
fn test_sandbox() -> Result<()> {
    let lua = Lua::new();

    lua.sandbox(true)?;

    lua.load("global = 123").exec()?;
    let n: i32 = lua.load("return global").eval()?;
    assert_eq!(n, 123);
    assert_eq!(lua.globals().get::<Option<i32>>("global")?, Some(123));

    // Threads should inherit "main" globals
    let f = lua.create_function(|lua, ()| lua.globals().get::<i32>("global"))?;
    let co = lua.create_thread(f.clone())?;
    assert_eq!(co.resume::<Option<i32>>(())?, Some(123));

    // Sandboxed threads should also inherit "main" globals
    let co = lua.create_thread(f)?;
    co.sandbox()?;
    assert_eq!(co.resume::<Option<i32>>(())?, Some(123));

    // collectgarbage should be restricted in sandboxed mode
    let collectgarbage = lua.globals().get::<Function>("collectgarbage")?;
    for arg in ["collect", "stop", "restart", "step", "isrunning"] {
        let err = collectgarbage.call::<()>(arg).err().unwrap().to_string();
        assert!(err.contains("collectgarbage called with invalid option"));
    }
    assert!(collectgarbage.call::<u64>("count").unwrap() > 0);

    lua.sandbox(false)?;

    // Previously set variable `global` should be cleared now
    assert_eq!(lua.globals().get::<Option<i32>>("global")?, None);

    // Readonly flags should be cleared as well
    let table = lua.globals().get::<Table>("table")?;
    table.set("test", "test")?;

    // collectgarbage should work now
    for arg in ["collect", "stop", "restart", "count", "step", "isrunning"] {
        collectgarbage.call::<()>(arg).unwrap();
    }

    Ok(())
}

#[test]
fn test_sandbox_safeenv() -> Result<()> {
    let lua = Lua::new();

    lua.sandbox(true)?;
    lua.globals().set("state", lua.create_table()?)?;
    lua.globals().set_safeenv(false);
    lua.load("state.a = 123").exec()?;
    let a: i32 = lua.load("state.a = 321; return state.a").eval()?;
    assert_eq!(a, 321);

    Ok(())
}

#[test]
fn test_sandbox_nolibs() -> Result<()> {
    let lua = Lua::new_with(StdLib::NONE, LuaOptions::default()).unwrap();

    lua.sandbox(true)?;
    lua.load("global = 123").exec()?;
    let n: i32 = lua.load("return global").eval()?;
    assert_eq!(n, 123);
    assert_eq!(lua.globals().get::<Option<i32>>("global")?, Some(123));

    lua.sandbox(false)?;
    assert_eq!(lua.globals().get::<Option<i32>>("global")?, None);

    Ok(())
}

#[test]
fn test_sandbox_threads() -> Result<()> {
    let lua = Lua::new();

    let f = lua.create_function(|lua, v: Value| lua.globals().set("global", v))?;

    let co = lua.create_thread(f.clone())?;
    co.resume::<()>(321)?;
    // The main state should see the `global` variable (as the thread is not sandboxed)
    assert_eq!(lua.globals().get::<Option<i32>>("global")?, Some(321));

    let co = lua.create_thread(f.clone())?;
    co.sandbox()?;
    co.resume::<()>(123)?;
    // The main state should see the previous `global` value (as the thread is sandboxed)
    assert_eq!(lua.globals().get::<Option<i32>>("global")?, Some(321));

    // Try to reset the (sandboxed) thread
    co.reset(f)?;
    co.resume::<()>(111)?;
    assert_eq!(lua.globals().get::<Option<i32>>("global")?, Some(111));

    Ok(())
}

#[test]
fn test_interrupts() -> Result<()> {
    let lua = Lua::new();

    let interrupts_count = Arc::new(AtomicU64::new(0));
    let interrupts_count2 = interrupts_count.clone();

    lua.set_interrupt(move |_| {
        interrupts_count2.fetch_add(1, Ordering::Relaxed);
        Ok(VmState::Continue)
    });
    let f = lua
        .load(
            r#"
        local x = 2 + 3
        local y = x * 63
        local z = string.len(x..", "..y)
    "#,
        )
        .into_function()?;
    f.call::<()>(())?;

    assert!(interrupts_count.load(Ordering::Relaxed) > 0);

    //
    // Test yields from interrupt
    //
    let yield_count = Arc::new(AtomicU64::new(0));
    let yield_count2 = yield_count.clone();
    lua.set_interrupt(move |_| {
        if yield_count2.fetch_add(1, Ordering::Relaxed) == 1 {
            return Ok(VmState::Yield);
        }
        Ok(VmState::Continue)
    });
    let co = lua.create_thread(
        lua.load(
            r#"
            local a = {1, 2, 3}
            local b = 0
            for _, x in ipairs(a) do b += x end
            return b
        "#,
        )
        .into_function()?,
    )?;
    co.resume::<()>(())?;
    assert!(co.is_resumable());
    let result: i32 = co.resume(())?;
    assert_eq!(result, 6);
    assert_eq!(yield_count.load(Ordering::Relaxed), 7);
    assert!(co.is_finished());

    // Test no yielding at non-yieldable points
    yield_count.store(0, Ordering::Relaxed);
    let co = lua.create_thread(lua.create_function(|lua, arg: Value| {
        (lua.load("return (function(x) return x end)(...)")).call::<Value>(arg)
    })?)?;
    let res = co.resume::<String>("abc")?;
    assert_eq!(res, "abc".to_string());
    assert_eq!(yield_count.load(Ordering::Relaxed), 3);

    //
    // Test errors in interrupts
    //
    lua.set_interrupt(|_| Err(Error::runtime("error from interrupt")));
    match f.call::<()>(()) {
        Err(Error::RuntimeError(ref msg)) => assert_eq!(msg, "error from interrupt"),
        res => panic!("expected `RuntimeError` with a specific message, got {res:?}"),
    }

    lua.remove_interrupt();

    Ok(())
}

#[test]
fn test_fflags() {
    // We cannot really on any particular feature flag to be present
    assert!(Lua::set_fflag("UnknownFlag", true).is_err());
}

#[cfg(feature = "luau-jit")]
#[test]
fn test_jit_inliner() -> Result<()> {
    let lua = Lua::new();
    lua.set_jit_options(mlua::JitOptions::new().set_inliner(true));

    // An inlinable helper called in a hot loop.
    let sum = lua
        .load(
            r#"
            local function add(a, b)
                return a + b
            end
            local sum = 0
            for i = 1, 1000 do
                sum = add(sum, i)
            end
            return sum
        "#,
        )
        .eval::<i64>()?;
    assert_eq!(sum, 500500);

    Ok(())
}

#[test]
fn test_loadstring() -> Result<()> {
    let lua = Lua::new();

    let f = lua.load(r#"loadstring("return 123")"#).eval::<Function>()?;
    assert_eq!(f.call::<i32>(())?, 123);

    let err = lua
        .load(r#"loadstring("retur 123", "chunk")"#) // typos:ignore
        .exec()
        .err()
        .unwrap();
    assert!(err.to_string().contains(
        r#"syntax error: [string "chunk"]:1: Incomplete statement: expected assignment or a function call"#
    ));

    Ok(())
}

#[test]
fn test_typeof_error() -> Result<()> {
    let lua = Lua::new();

    let err = Error::runtime("just a test error");
    let res = lua.load("return typeof(...)").call::<String>(err)?;
    assert_eq!(res, "error");

    Ok(())
}

#[test]
fn test_memory_category() -> Result<()> {
    let lua = Lua::new();

    lua.set_memory_category("main").unwrap();

    // Invalid category names should be rejected
    let err = lua.set_memory_category("invalid$");
    assert!(err.is_err());

    for i in 0..254 {
        let name = format!("category_{}", i);
        lua.set_memory_category(&name).unwrap();
    }
    // 255th category should fail
    let err = lua.set_memory_category("category_254");
    assert!(err.is_err());

    Ok(())
}

#[test]
fn test_heap_dump() -> Result<()> {
    let lua = Lua::new();

    // Assign a new memory category and create few objects
    lua.set_memory_category("test_category")?;
    let _t = lua.create_table()?;
    let _ud = lua.create_any_userdata("hello, world")?;

    let dump = lua.heap_dump()?;

    assert!(dump.size() > 0);
    let size_by_category = dump.size_by_category();
    assert_eq!(size_by_category.len(), 2);
    assert!(size_by_category.contains_key("test_category"));
    assert!(size_by_category["main"] < dump.size());

    // Check size by type within the category
    let size_by_type = dump.size_by_type(Some("test_category"));
    assert!(!size_by_type.is_empty());
    assert!(size_by_type.contains_key("table"));
    assert!(size_by_type.contains_key("userdata"));
    // Try non-existent category
    let size_by_type2 = dump.size_by_type(Some("non_existent_category"));
    assert!(size_by_type2.is_empty());
    // Remove category filter
    let size_by_type_all = dump.size_by_type(None);
    assert!(size_by_type.len() < size_by_type_all.len());

    // Check size by userdata type within the category
    let size_by_udtype = dump.size_by_userdata(Some("test_category"));
    assert_eq!(size_by_udtype.len(), 1);
    assert!(size_by_udtype.contains_key("&str"));
    assert_eq!(size_by_udtype["&str"].0, 1);
    // Try non-existent category
    let size_by_udtype2 = dump.size_by_userdata(Some("non_existent_category"));
    assert!(size_by_udtype2.is_empty());
    // Remove category filter
    let size_by_udtype_all = dump.size_by_userdata(None);
    assert!(size_by_udtype.len() < size_by_udtype_all.len());

    Ok(())
}

#[test]
fn test_integer64_type() -> Result<()> {
    let lua = Lua::new();

    _ = Lua::set_fflag("LuauIntegerType2", true);

    let integer_lib = lua.globals().get::<Table>("integer")?;
    let n = integer_lib.call_function::<i64>("create", 42)?;
    assert_eq!(n, 42);

    let n: i64 = lua.load("return 42i").eval()?;
    assert_eq!(n, 42);
    let n: i64 = lua.load("return -42i").eval()?;
    assert_eq!(n, -42);

    Ok(())
}

#[path = "luau/require.rs"]
mod require;

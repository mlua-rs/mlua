use std::sync::Arc;

use mlua::state::{GcIncParams, GcMode};
use mlua::{Error, Function, Lua, MultiValue, Result, Table, UserData};

#[cfg(any(feature = "lua54", feature = "lua55"))]
use mlua::state::GcGenParams;

#[test]
fn test_memory_limit() -> Result<()> {
    let lua = Lua::new();

    let initial_memory = lua.used_memory();
    assert!(
        initial_memory > 0,
        "used_memory reporting is wrong, lua uses memory for stdlib"
    );

    let f = lua
        .load("local t = {}; for i = 1,10000 do t[i] = i end")
        .into_function()?;
    f.call::<()>(()).expect("should trigger no memory limit");

    if cfg!(feature = "luajit") && lua.set_memory_limit(0).is_err() {
        // seems this luajit version does not support memory limit
        return Ok(());
    }

    lua.set_memory_limit(initial_memory + 10000)?;
    match f.call::<()>(()) {
        Err(Error::MemoryError(_)) => {}
        something_else => panic!("did not trigger memory error: {:?}", something_else),
    };

    lua.set_memory_limit(0)?;
    f.call::<()>(()).expect("should trigger no memory limit");

    // Test memory limit during chunk loading
    lua.set_memory_limit(1024)?;
    match lua.load(format!("return '{}'", "x".repeat(4096))).into_function() {
        Err(Error::MemoryError(_)) => {}
        _ => panic!("did not trigger memory error"),
    };

    // Callback cleanup must not panic if the reference stack cannot grow under a memory limit.
    for nrefs in 0..64 {
        let lua = Lua::new();
        let f = lua.create_function(|lua, value: i32| {
            lua.set_memory_limit(1)?;
            Ok(value)
        })?;
        let _refs: Vec<_> = (0..nrefs).map(|_| lua.globals()).collect();
        assert_eq!(f.call::<i32>(42)?, 42);
        lua.set_memory_limit(0)?;
    }

    Ok(())
}

#[test]
fn test_result_reference_exhaustion() -> Result<()> {
    let lua = Lua::new();
    if cfg!(feature = "luajit") && lua.set_memory_limit(0).is_err() {
        return Ok(());
    }
    let f: Function = lua
        .load("local t = {}; return function() return t, t end")
        .eval()?;
    f.call::<MultiValue>(())?; // Warm the call stack before limiting allocations.
    lua.gc_stop();
    lua.set_memory_limit(1)?;

    let mut refs = Vec::new();
    loop {
        match f.call::<Table>(()) {
            Ok(table) => refs.push(table),
            Err(Error::StackError) => break,
            Err(err) => return Err(err),
        }
    }

    // The first result can be retained, but the second must fail and release the first.
    drop(refs.pop().unwrap());
    assert!(matches!(f.call::<MultiValue>(()), Err(Error::StackError)));
    f.call::<Table>(())?;
    lua.set_memory_limit(0)?;
    assert_eq!(f.call::<MultiValue>(())?.len(), 2);
    Ok(())
}

#[test]
fn test_memory_limit_thread() -> Result<()> {
    let lua = Lua::new();

    let f = lua
        .load("local t = {}; for i = 1,10000 do t[i] = i end")
        .into_function()?;

    if cfg!(feature = "luajit") && lua.set_memory_limit(0).is_err() {
        // seems this luajit version does not support memory limit
        return Ok(());
    }

    let thread = lua.create_thread(f)?;
    lua.set_memory_limit(lua.used_memory() + 10000)?;
    match thread.resume::<()>(()) {
        Err(Error::MemoryError(_)) => {}
        something_else => panic!("did not trigger memory error: {:?}", something_else),
    };

    Ok(())
}

// Creates a string of `len` bytes that is not interned yet, returns `false` on a memory error
fn try_create_string(lua: &Lua, len: usize) -> bool {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);

    let mut bytes = vec![b'x'; len.max(8)];
    bytes.truncate(len);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed).to_le_bytes();
    let n = len.min(8);
    bytes[..n].copy_from_slice(&seq[..n]);
    match lua.create_string(&bytes) {
        Ok(_) => true,
        Err(Error::MemoryError(_)) => false,
        Err(err) => panic!("unexpected error: {err:?}"),
    }
}

#[test]
fn test_allocation_limit() -> Result<()> {
    let lua = Lua::new();

    if cfg!(feature = "luajit") && lua.set_allocation_limit(0).is_err() {
        // seems this luajit version does not support memory limit
        return Ok(());
    }

    // No limit by default
    assert_eq!(lua.set_allocation_limit(0)?, 0);
    assert!(try_create_string(&lua, 1 << 24));
    lua.gc_collect()?;

    const LIMIT: usize = 64 * 1024;
    assert_eq!(lua.set_allocation_limit(LIMIT)?, 0);
    assert!(try_create_string(&lua, LIMIT / 2));
    assert!(!try_create_string(&lua, LIMIT));

    // The largest string that fits grows with the limit byte for byte
    let largest_string = |lua: &Lua| {
        let (mut lo, mut hi) = (LIMIT / 2, 2 * LIMIT);
        while hi - lo > 1 {
            let mid = (lo + hi) / 2;
            if try_create_string(lua, mid) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        assert!(try_create_string(lua, lo));
        assert!(!try_create_string(lua, lo + 1));
        lo
    };
    let largest = largest_string(&lua);
    assert!(largest < LIMIT);
    lua.set_allocation_limit(LIMIT + 64)?;
    assert_eq!(largest_string(&lua), largest + 64);
    lua.set_allocation_limit(LIMIT)?;

    // Growing a table past the limit fails
    let grow = lua
        .load("local t = {}; for i = 1, 100000 do t[i] = i end; return #t")
        .into_function()?;
    match grow.call::<usize>(()) {
        Err(Error::MemoryError(_)) => {}
        something_else => panic!("did not trigger memory error: {:?}", something_else),
    }

    // The error can be caught by `pcall` and Lua keeps working
    let (ok, len) = lua
        .load(
            r#"
            local ok = pcall(string.rep, "x", 2 * 65536)
            return ok, #string.rep("x", 1024)
        "#,
        )
        .eval::<(bool, usize)>()?;
    assert!(!ok);
    assert_eq!(len, 1024);
    assert_eq!(lua.load("return 1 + 1").eval::<i32>()?, 2);

    assert_eq!(lua.set_allocation_limit(0)?, LIMIT);
    assert_eq!(grow.call::<usize>(())?, 100000);

    Ok(())
}

#[test]
fn test_allocation_limit_with_memory_limit() -> Result<()> {
    let lua = Lua::new();

    if cfg!(feature = "luajit") && lua.set_allocation_limit(0).is_err() {
        // seems this luajit version does not support memory limit
        return Ok(());
    }

    const LIMIT: usize = 64 * 1024;
    lua.set_allocation_limit(LIMIT)?;
    let memory_limit = lua.used_memory() + (1 << 20);
    lua.set_memory_limit(memory_limit)?;

    // Each limit applies on its own
    assert!(try_create_string(&lua, LIMIT / 2));
    assert!(!try_create_string(&lua, 2 * LIMIT));
    assert_eq!(lua.set_allocation_limit(0)?, LIMIT);
    assert!(try_create_string(&lua, 2 * LIMIT));
    lua.gc_collect()?;
    assert!(!try_create_string(&lua, 2 << 20));

    // Removing the memory limit keeps the allocation limit
    lua.set_allocation_limit(LIMIT)?;
    assert_eq!(lua.set_memory_limit(0)?, memory_limit);
    assert!(try_create_string(&lua, LIMIT / 2));
    assert!(!try_create_string(&lua, 2 * LIMIT));

    Ok(())
}

#[test]
fn test_gc_control() -> Result<()> {
    let lua = Lua::new();
    let globals = lua.globals();

    #[cfg(any(feature = "lua55", feature = "lua54"))]
    {
        assert!(matches!(
            lua.gc_set_mode(GcMode::Generational(GcGenParams::default())),
            GcMode::Incremental(_)
        ));
        assert!(matches!(
            lua.gc_set_mode(GcMode::Incremental(GcIncParams::default())),
            GcMode::Generational(_)
        ));
    }

    #[cfg(any(
        feature = "lua55",
        feature = "lua54",
        feature = "lua53",
        feature = "lua52",
        feature = "luau"
    ))]
    {
        assert!(lua.gc_is_running());
        lua.gc_stop();
        assert!(!lua.gc_is_running());
        lua.gc_restart();
        assert!(lua.gc_is_running());
    }

    assert!(matches!(
        lua.gc_set_mode(GcMode::Incremental({
            let p = GcIncParams::default().step_multiplier(100);
            #[cfg(not(feature = "luau"))]
            let p = p.pause(200);
            #[cfg(feature = "luau")]
            let p = p.goal(200);
            p
        })),
        GcMode::Incremental(_)
    ));

    struct MyUserdata(#[allow(unused)] Arc<()>);
    impl UserData for MyUserdata {}

    let rc = Arc::new(());
    globals.set("userdata", lua.create_userdata(MyUserdata(rc.clone()))?)?;
    globals.raw_remove("userdata")?;

    assert_eq!(Arc::strong_count(&rc), 2);
    lua.gc_collect()?;
    lua.gc_collect()?;
    assert_eq!(Arc::strong_count(&rc), 1);

    Ok(())
}

#[cfg(any(feature = "lua54", feature = "lua55"))]
#[test]
fn test_gc_set_mode_in_finalizer() -> Result<()> {
    use std::sync::atomic::{AtomicBool, Ordering};

    let lua = Lua::new();

    // While finalizers are running, the GC is internally stopped and `lua_gc` rejects all
    // options
    let switched = Arc::new(AtomicBool::new(false));
    let switched2 = switched.clone();
    let finalizer = lua.create_function(move |lua, ()| {
        let mode = lua.gc_set_mode(GcMode::Generational(GcGenParams::default()));
        switched2.store(matches!(mode, GcMode::Generational(_)), Ordering::Relaxed);
        Ok(())
    })?;
    lua.globals().set("finalizer", finalizer)?;
    lua.load("setmetatable({}, { __gc = finalizer })").exec()?;
    lua.globals().raw_remove("finalizer")?;

    lua.gc_collect()?;
    lua.gc_collect()?;
    assert!(
        switched.load(Ordering::Relaxed),
        "gc_set_mode did not complete in finalizer"
    );

    Ok(())
}

#[cfg(any(feature = "lua53", feature = "lua52"))]
#[test]
fn test_gc_error() {
    use mlua::Error;

    let lua = Lua::new();
    match lua
        .load(
            r#"
            val = nil
            table = {}
            setmetatable(table, {
                __gc = function()
                    error("gcwascalled")
                end
            })
            table = nil
            collectgarbage("collect")
    "#,
        )
        .exec()
    {
        Err(Error::GarbageCollectorError(_)) => {}
        Err(e) => panic!("__gc error did not result in correct error, instead: {}", e),
        Ok(()) => panic!("__gc error did not result in error"),
    }
}

#![cfg(feature = "luau")]

use std::env;
use std::fmt::Debug;
use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use mlua::{Compiler, CoverageInfo, Error, Lua, Result, Table, ThreadStatus, Value, VmState};

#[test]
fn test_require() -> Result<()> {
    let lua = Lua::new();

    let temp_dir = tempfile::tempdir().unwrap();
    fs::write(
        temp_dir.path().join("module.luau"),
        r#"
        counter = (counter or 0) + 1
        return {
            counter = counter,
            error = function() error("test") end,
        }
    "#,
    )?;

    env::set_var("LUAU_PATH", temp_dir.path().join("?.luau"));
    lua.load(
        r#"
        local module = require("module")
        assert(module.counter == 1)
        module = require("module")
        assert(module.counter == 1)

        local ok, err = pcall(module.error)
        assert(not ok and string.find(err, "module.luau") ~= nil)
    "#,
    )
    .exec()
}

#[test]
fn test_vectors() -> Result<()> {
    let lua = Lua::new();

    let v: [f32; 3] = lua.load("vector(1, 2, 3) + vector(3, 2, 1)").eval()?;
    assert_eq!(v, [4.0, 4.0, 4.0]);

    // Test vector methods
    lua.load(
        r#"
        local v = vector(1, 2, 3)
        assert(v.x == 1)
        assert(v.y == 2)
        assert(v.z == 3)
    "#,
    )
    .exec()?;

    // Test vector methods (fastcall)
    lua.load(
        r#"
        local v = vector(1, 2, 3)
        assert(v.x == 1)
        assert(v.y == 2)
        assert(v.z == 3)
    "#,
    )
    .set_compiler(Compiler::new().set_vector_ctor(Some("vector".to_string())))
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
    match catch_unwind(AssertUnwindSafe(|| t.set_metatable(None))) {
        Ok(_) => panic!("expected panic, got nothing"),
        Err(_) => {}
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
    assert_eq!(lua.globals().get::<_, Option<i32>>("global")?, Some(123));

    // Threads should inherit "main" globals
    let f = lua.create_function(|lua, ()| lua.globals().get::<_, i32>("global"))?;
    let co = lua.create_thread(f.clone())?;
    assert_eq!(co.resume::<_, Option<i32>>(())?, Some(123));

    // Sandboxed threads should also inherit "main" globals
    let co = lua.create_thread(f)?;
    co.sandbox()?;
    assert_eq!(co.resume::<_, Option<i32>>(())?, Some(123));

    lua.sandbox(false)?;

    // Previously set variable `global` should be cleared now
    assert_eq!(lua.globals().get::<_, Option<i32>>("global")?, None);

    // Readonly flags should be cleared as well
    let table = lua.globals().get::<_, Table>("table")?;
    table.set("test", "test")?;

    Ok(())
}

#[test]
fn test_sandbox_threads() -> Result<()> {
    let lua = Lua::new();

    let f = lua.create_function(|lua, v: Value| lua.globals().set("global", v))?;

    let co = lua.create_thread(f.clone())?;
    co.resume(321)?;
    // The main state should see the `global` variable (as the thread is not sandboxed)
    assert_eq!(lua.globals().get::<_, Option<i32>>("global")?, Some(321));

    let co = lua.create_thread(f.clone())?;
    co.sandbox()?;
    co.resume(123)?;
    // The main state should see the previous `global` value (as the thread is sandboxed)
    assert_eq!(lua.globals().get::<_, Option<i32>>("global")?, Some(321));

    // Try to reset the (sandboxed) thread
    co.reset(f)?;
    co.resume(111)?;
    assert_eq!(lua.globals().get::<_, Option<i32>>("global")?, Some(111));

    Ok(())
}

#[test]
fn test_interrupts() -> Result<()> {
    let lua = Lua::new();

    let interrupts_count = Arc::new(AtomicU64::new(0));
    let interrupts_count2 = interrupts_count.clone();

    lua.set_interrupt(move || {
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
    f.call(())?;

    assert!(interrupts_count.load(Ordering::Relaxed) > 0);

    //
    // Test yields from interrupt
    //
    let yield_count = Arc::new(AtomicU64::new(0));
    let yield_count2 = yield_count.clone();
    lua.set_interrupt(move || {
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
    co.resume(())?;
    assert_eq!(co.status(), ThreadStatus::Resumable);
    let result: i32 = co.resume(())?;
    assert_eq!(result, 6);
    assert_eq!(yield_count.load(Ordering::Relaxed), 7);
    assert_eq!(co.status(), ThreadStatus::Unresumable);

    //
    // Test errors in interrupts
    //
    lua.set_interrupt(|| Err(Error::RuntimeError("error from interrupt".into())));
    match f.call::<_, ()>(()) {
        Err(Error::CallbackError { cause, .. }) => match *cause {
            Error::RuntimeError(ref m) if m == "error from interrupt" => {}
            ref e => panic!("expected RuntimeError with a specific message, got {:?}", e),
        },
        r => panic!("expected CallbackError, got {:?}", r),
    }

    lua.remove_interrupt();

    Ok(())
}

#[test]
fn test_coverage() -> Result<()> {
    let lua = Lua::new();

    lua.set_compiler(Compiler::default().set_coverage_level(1));

    let f = lua
        .load(
            r#"local v = vector(1, 2, 3)
        assert(v.x == 1 and v.y == 2 and v.z == 3)

        function abc(i)
            if i < 5 then
                return 0
            else
                return 1
            end
        end

        (function()
            (function() abc(10) end)()
        end)()
        "#,
        )
        .into_function()?;

    f.call(())?;

    let mut report = Vec::new();
    f.coverage(|cov| {
        report.push(cov);
    });

    assert_eq!(
        report[0],
        CoverageInfo {
            function: None,
            line_defined: 1,
            depth: 0,
            hits: vec![-1, 1, 1, -1, 1, -1, -1, -1, -1, -1, -1, -1, 1, -1, -1, -1],
        }
    );
    assert_eq!(
        report[1],
        CoverageInfo {
            function: Some("abc".into()),
            line_defined: 4,
            depth: 1,
            hits: vec![-1, -1, -1, -1, -1, 1, 0, -1, 1, -1, -1, -1, -1, -1, -1, -1],
        }
    );
    assert_eq!(
        report[2],
        CoverageInfo {
            function: None,
            line_defined: 12,
            depth: 1,
            hits: vec![-1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, 1, -1, -1],
        }
    );
    assert_eq!(
        report[3],
        CoverageInfo {
            function: None,
            line_defined: 13,
            depth: 2,
            hits: vec![-1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, 1, -1, -1],
        }
    );

    Ok(())
}

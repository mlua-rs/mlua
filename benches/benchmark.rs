#![cfg_attr(
    all(feature = "luajit", target_os = "macos", target_arch = "x86_64"),
    feature(link_args)
)]

#[cfg_attr(
    all(feature = "luajit", target_os = "macos", target_arch = "x86_64"),
    link_args = "-pagezero_size 10000 -image_base 100000000",
    allow(unused_attributes)
)]
extern "system" {}

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use std::time::Duration;
use tokio::runtime::Runtime;
use tokio::task;

use mlua::prelude::*;

fn collect_gc_twice(lua: &Lua) {
    lua.gc_collect().unwrap();
    lua.gc_collect().unwrap();
}

fn create_table(c: &mut Criterion) {
    let lua = Lua::new();

    c.bench_function("create [table empty]", |b| {
        b.iter_batched(
            || collect_gc_twice(&lua),
            |_| {
                lua.create_table().unwrap();
            },
            BatchSize::SmallInput,
        );
    });
}

fn create_array(c: &mut Criterion) {
    let lua = Lua::new();

    c.bench_function("create [array] 10", |b| {
        b.iter_batched(
            || collect_gc_twice(&lua),
            |_| {
                let table = lua.create_table().unwrap();
                for i in 1..=10 {
                    table.set(i, i).unwrap();
                }
            },
            BatchSize::SmallInput,
        );
    });
}

fn create_string_table(c: &mut Criterion) {
    let lua = Lua::new();

    c.bench_function("create [table string] 10", |b| {
        b.iter_batched(
            || collect_gc_twice(&lua),
            |_| {
                let table = lua.create_table().unwrap();
                for &s in &["1", "2", "3", "4", "5", "6", "7", "8", "9", "10"] {
                    let s = lua.create_string(s).unwrap();
                    table.set(s.clone(), s).unwrap();
                }
            },
            BatchSize::SmallInput,
        );
    });
}

fn call_lua_function(c: &mut Criterion) {
    let lua = Lua::new();

    c.bench_function("call Lua function [sum] 3 10", |b| {
        b.iter_batched_ref(
            || {
                collect_gc_twice(&lua);
                lua.load("function(a, b, c) return a + b + c end")
                    .eval::<LuaFunction>()
                    .unwrap()
            },
            |function| {
                for i in 0..10 {
                    let _result: i64 = function.call((i, i + 1, i + 2)).unwrap();
                }
            },
            BatchSize::SmallInput,
        );
    });
}

fn call_sum_callback(c: &mut Criterion) {
    let lua = Lua::new();
    let callback = lua
        .create_function(|_, (a, b, c): (i64, i64, i64)| Ok(a + b + c))
        .unwrap();
    lua.globals().set("callback", callback).unwrap();

    c.bench_function("call Rust callback [sum] 3 10", |b| {
        b.iter_batched_ref(
            || {
                collect_gc_twice(&lua);
                lua.load("function() for i = 1,10 do callback(i, i+1, i+2) end end")
                    .eval::<LuaFunction>()
                    .unwrap()
            },
            |function| {
                function.call::<_, ()>(()).unwrap();
            },
            BatchSize::SmallInput,
        );
    });
}

fn call_async_sum_callback(c: &mut Criterion) {
    let lua = Lua::new();
    let callback = lua
        .create_async_function(|_, (a, b, c): (i64, i64, i64)| async move {
            task::yield_now().await;
            Ok(a + b + c)
        })
        .unwrap();
    lua.globals().set("callback", callback).unwrap();

    c.bench_function("call async Rust callback [sum] 3 10", |b| {
        let rt = Runtime::new().unwrap();
        b.to_async(rt).iter_batched(
            || {
                collect_gc_twice(&lua);
                lua.load("function() for i = 1,10 do callback(i, i+1, i+2) end end")
                    .eval::<LuaFunction>()
                    .unwrap()
            },
            |function| async move {
                function.call_async::<_, ()>(()).await.unwrap();
            },
            BatchSize::SmallInput,
        );
    });
}

fn call_concat_callback(c: &mut Criterion) {
    let lua = Lua::new();
    let callback = lua
        .create_function(|_, (a, b): (LuaString, LuaString)| {
            Ok(format!("{}{}", a.to_str()?, b.to_str()?))
        })
        .unwrap();
    lua.globals().set("callback", callback).unwrap();

    c.bench_function("call Rust callback [concat string] 10", |b| {
        b.iter_batched_ref(
            || {
                collect_gc_twice(&lua);
                lua.load("function() for i = 1,10 do callback('a', tostring(i)) end end")
                    .eval::<LuaFunction>()
                    .unwrap()
            },
            |function| {
                function.call::<_, ()>(()).unwrap();
            },
            BatchSize::SmallInput,
        );
    });
}

fn create_registry_values(c: &mut Criterion) {
    let lua = Lua::new();

    c.bench_function("create [registry value] 10", |b| {
        b.iter_batched(
            || collect_gc_twice(&lua),
            |_| {
                for _ in 0..10 {
                    lua.create_registry_value(lua.pack(true).unwrap()).unwrap();
                }
                lua.expire_registry_values();
            },
            BatchSize::SmallInput,
        );
    });
}

fn create_userdata(c: &mut Criterion) {
    struct UserData(i64);
    impl LuaUserData for UserData {}

    let lua = Lua::new();

    c.bench_function("create [table userdata] 10", |b| {
        b.iter_batched(
            || collect_gc_twice(&lua),
            |_| {
                let table: LuaTable = lua.create_table().unwrap();
                for i in 1..11 {
                    table.set(i, UserData(i)).unwrap();
                }
            },
            BatchSize::SmallInput,
        );
    });
}

fn call_userdata_method(c: &mut Criterion) {
    struct UserData(i64);
    impl LuaUserData for UserData {
        fn add_methods<'lua, M: LuaUserDataMethods<'lua, Self>>(methods: &mut M) {
            methods.add_method("method", |_, this, ()| Ok(this.0));
        }
    }

    let lua = Lua::new();
    lua.globals().set("userdata", UserData(10)).unwrap();

    c.bench_function("call [userdata method] 10", |b| {
        b.iter_batched_ref(
            || {
                collect_gc_twice(&lua);
                lua.load("function() for i = 1,10 do userdata:method() end end")
                    .eval::<LuaFunction>()
                    .unwrap()
            },
            |function| {
                function.call::<_, ()>(()).unwrap();
            },
            BatchSize::SmallInput,
        );
    });
}

fn call_async_userdata_method(c: &mut Criterion) {
    #[derive(Clone, Copy)]
    struct UserData(i64);
    impl LuaUserData for UserData {
        fn add_methods<'lua, M: LuaUserDataMethods<'lua, Self>>(methods: &mut M) {
            methods.add_async_method("method", |_, this, ()| async move { Ok(this.0) });
        }
    }

    let lua = Lua::new();
    lua.globals().set("userdata", UserData(10)).unwrap();

    c.bench_function("call async [userdata method] 10", |b| {
        let rt = Runtime::new().unwrap();
        b.to_async(rt).iter_batched(
            || {
                collect_gc_twice(&lua);
                lua.load("function() for i = 1,10 do userdata:method() end end")
                    .eval::<LuaFunction>()
                    .unwrap()
            },
            |function| async move {
                function.call_async::<_, ()>(()).await.unwrap();
            },
            BatchSize::SmallInput,
        );
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(300)
        .measurement_time(Duration::from_secs(10))
        .noise_threshold(0.02);
    targets =
        create_table,
        create_array,
        create_string_table,
        call_lua_function,
        call_sum_callback,
        call_async_sum_callback,
        call_concat_callback,
        create_registry_values,
        create_userdata,
        call_userdata_method,
        call_async_userdata_method,
}

criterion_main!(benches);

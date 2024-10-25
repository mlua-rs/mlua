use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use tokio::runtime::Runtime;
use tokio::task;

use mlua::prelude::*;

fn collect_gc_twice(lua: &Lua) {
    lua.gc_collect().unwrap();
    lua.gc_collect().unwrap();
}

fn table_create_empty(c: &mut Criterion) {
    let lua = Lua::new();

    c.bench_function("table [create empty]", |b| {
        b.iter_batched(
            || collect_gc_twice(&lua),
            |_| {
                lua.create_table().unwrap();
            },
            BatchSize::SmallInput,
        );
    });
}

fn table_create_array(c: &mut Criterion) {
    let lua = Lua::new();

    c.bench_function("table [create array]", |b| {
        b.iter_batched(
            || collect_gc_twice(&lua),
            |_| {
                lua.create_sequence_from(1..=10).unwrap();
            },
            BatchSize::SmallInput,
        );
    });
}

fn table_create_hash(c: &mut Criterion) {
    let lua = Lua::new();

    c.bench_function("table [create hash]", |b| {
        b.iter_batched(
            || collect_gc_twice(&lua),
            |_| {
                lua.create_table_from(
                    ["1", "2", "3", "4", "5", "6", "7", "8", "9", "10"]
                        .into_iter()
                        .map(|s| (s, s)),
                )
                .unwrap();
            },
            BatchSize::SmallInput,
        );
    });
}

fn table_get_set(c: &mut Criterion) {
    let lua = Lua::new();

    c.bench_function("table [get and set]", |b| {
        b.iter_batched(
            || {
                collect_gc_twice(&lua);
                lua.create_table().unwrap()
            },
            |table| {
                for (i, s) in ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"]
                    .into_iter()
                    .enumerate()
                {
                    table.raw_set(s, i).unwrap();
                    assert_eq!(table.raw_get::<usize>(s).unwrap(), i);
                }
            },
            BatchSize::SmallInput,
        );
    });
}

fn table_traversal_pairs(c: &mut Criterion) {
    let lua = Lua::new();

    c.bench_function("table [traversal pairs]", |b| {
        b.iter_batched(
            || lua.globals(),
            |globals| {
                for kv in globals.pairs::<String, LuaValue>() {
                    let (_k, _v) = kv.unwrap();
                }
            },
            BatchSize::SmallInput,
        );
    });
}

fn table_traversal_for_each(c: &mut Criterion) {
    let lua = Lua::new();

    c.bench_function("table [traversal for_each]", |b| {
        b.iter_batched(
            || lua.globals(),
            |globals| globals.for_each::<String, LuaValue>(|_k, _v| Ok(())),
            BatchSize::SmallInput,
        );
    });
}

fn table_traversal_sequence(c: &mut Criterion) {
    let lua = Lua::new();

    let table = lua.create_sequence_from(1..1000).unwrap();

    c.bench_function("table [traversal sequence]", |b| {
        b.iter_batched(
            || table.clone(),
            |table| {
                for v in table.sequence_values::<i32>() {
                    let _i = v.unwrap();
                }
            },
            BatchSize::SmallInput,
        );
    });
}

fn function_create(c: &mut Criterion) {
    let lua = Lua::new();

    c.bench_function("function [create Rust]", |b| {
        b.iter_batched(
            || collect_gc_twice(&lua),
            |_| {
                lua.create_function(|_, ()| Ok(123)).unwrap();
            },
            BatchSize::SmallInput,
        );
    });
}

fn function_call_sum(c: &mut Criterion) {
    let lua = Lua::new();

    let sum = lua
        .create_function(|_, (a, b, c): (i64, i64, i64)| Ok(a + b - c))
        .unwrap();

    c.bench_function("function [call Rust sum]", |b| {
        b.iter_batched(
            || collect_gc_twice(&lua),
            |_| {
                assert_eq!(sum.call::<i64>((10, 20, 30)).unwrap(), 0);
            },
            BatchSize::SmallInput,
        );
    });
}

fn function_call_lua_sum(c: &mut Criterion) {
    let lua = Lua::new();

    let sum = lua
        .load("function(a, b, c) return a + b - c end")
        .eval::<LuaFunction>()
        .unwrap();

    c.bench_function("function [call Lua sum]", |b| {
        b.iter_batched(
            || collect_gc_twice(&lua),
            |_| {
                assert_eq!(sum.call::<i64>((10, 20, 30)).unwrap(), 0);
            },
            BatchSize::SmallInput,
        );
    });
}

fn function_call_concat(c: &mut Criterion) {
    let lua = Lua::new();

    let concat = lua
        .create_function(|_, (a, b): (LuaString, LuaString)| Ok(format!("{}{}", a.to_str()?, b.to_str()?)))
        .unwrap();
    let i = AtomicUsize::new(0);

    c.bench_function("function [call Rust concat string]", |b| {
        b.iter_batched(
            || {
                collect_gc_twice(&lua);
                i.fetch_add(1, Ordering::Relaxed)
            },
            |i| {
                assert_eq!(concat.call::<LuaString>(("num:", i)).unwrap(), format!("num:{i}"));
            },
            BatchSize::SmallInput,
        );
    });
}

fn function_call_lua_concat(c: &mut Criterion) {
    let lua = Lua::new();

    let concat = lua
        .load("function(a, b) return a..b end")
        .eval::<LuaFunction>()
        .unwrap();
    let i = AtomicUsize::new(0);

    c.bench_function("function [call Lua concat string]", |b| {
        b.iter_batched(
            || {
                collect_gc_twice(&lua);
                i.fetch_add(1, Ordering::Relaxed)
            },
            |i| {
                assert_eq!(concat.call::<LuaString>(("num:", i)).unwrap(), format!("num:{i}"));
            },
            BatchSize::SmallInput,
        );
    });
}

fn function_async_call_sum(c: &mut Criterion) {
    let options = LuaOptions::new().thread_pool_size(1024);
    let lua = Lua::new_with(LuaStdLib::ALL_SAFE, options).unwrap();

    let sum = lua
        .create_async_function(|_, (a, b, c): (i64, i64, i64)| async move {
            task::yield_now().await;
            Ok(a + b - c)
        })
        .unwrap();

    c.bench_function("function [async call Rust sum]", |b| {
        let rt = Runtime::new().unwrap();
        b.to_async(rt).iter_batched(
            || collect_gc_twice(&lua),
            |_| async {
                assert_eq!(sum.call_async::<i64>((10, 20, 30)).await.unwrap(), 0);
            },
            BatchSize::SmallInput,
        );
    });
}

fn registry_value_create(c: &mut Criterion) {
    let lua = Lua::new();
    lua.gc_stop();

    c.bench_function("registry value [create]", |b| {
        b.iter_batched(
            || collect_gc_twice(&lua),
            |_| lua.create_registry_value("hello").unwrap(),
            BatchSize::SmallInput,
        );
    });
}

fn registry_value_get(c: &mut Criterion) {
    let lua = Lua::new();
    lua.gc_stop();

    let value = lua.create_registry_value("hello").unwrap();

    c.bench_function("registry value [get]", |b| {
        b.iter_batched(
            || collect_gc_twice(&lua),
            |_| {
                assert_eq!(lua.registry_value::<LuaString>(&value).unwrap(), "hello");
            },
            BatchSize::SmallInput,
        );
    });
}

fn userdata_create(c: &mut Criterion) {
    struct UserData(#[allow(unused)] i64);
    impl LuaUserData for UserData {}

    let lua = Lua::new();

    c.bench_function("userdata [create]", |b| {
        b.iter_batched(
            || collect_gc_twice(&lua),
            |_| {
                lua.create_userdata(UserData(123)).unwrap();
            },
            BatchSize::SmallInput,
        );
    });
}

fn userdata_call_index(c: &mut Criterion) {
    struct UserData(#[allow(unused)] i64);
    impl LuaUserData for UserData {
        fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
            methods.add_meta_method(LuaMetaMethod::Index, move |_, _, key: LuaString| Ok(key));
        }
    }

    let lua = Lua::new();
    let ud = lua.create_userdata(UserData(123)).unwrap();
    let index = lua
        .load("function(ud) return ud.test end")
        .eval::<LuaFunction>()
        .unwrap();

    c.bench_function("userdata [call index]", |b| {
        b.iter_batched(
            || collect_gc_twice(&lua),
            |_| {
                assert_eq!(index.call::<LuaString>(&ud).unwrap(), "test");
            },
            BatchSize::SmallInput,
        );
    });
}

fn userdata_call_method(c: &mut Criterion) {
    struct UserData(i64);
    impl LuaUserData for UserData {
        fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
            methods.add_method("add", |_, this, i: i64| Ok(this.0 + i));
        }
    }

    let lua = Lua::new();
    let ud = lua.create_userdata(UserData(123)).unwrap();
    let method = lua
        .load("function(ud, i) return ud:add(i) end")
        .eval::<LuaFunction>()
        .unwrap();
    let i = AtomicUsize::new(0);

    c.bench_function("userdata [call method]", |b| {
        b.iter_batched(
            || {
                collect_gc_twice(&lua);
                i.fetch_add(1, Ordering::Relaxed)
            },
            |i| {
                assert_eq!(method.call::<usize>((&ud, i)).unwrap(), 123 + i);
            },
            BatchSize::SmallInput,
        );
    });
}

fn userdata_async_call_method(c: &mut Criterion) {
    struct UserData(i64);
    impl LuaUserData for UserData {
        fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
            methods.add_async_method("add", |_, this, i: i64| async move {
                task::yield_now().await;
                Ok(this.0 + i)
            });
        }
    }

    let options = LuaOptions::new().thread_pool_size(1024);
    let lua = Lua::new_with(LuaStdLib::ALL_SAFE, options).unwrap();
    let ud = lua.create_userdata(UserData(123)).unwrap();
    let method = lua
        .load("function(ud, i) return ud:add(i) end")
        .eval::<LuaFunction>()
        .unwrap();
    let i = AtomicUsize::new(0);

    c.bench_function("userdata [async call method] 10", |b| {
        let rt = Runtime::new().unwrap();
        b.to_async(rt).iter_batched(
            || {
                collect_gc_twice(&lua);
                (method.clone(), ud.clone(), i.fetch_add(1, Ordering::Relaxed))
            },
            |(method, ud, i)| async move {
                assert_eq!(method.call_async::<usize>((ud, i)).await.unwrap(), 123 + i);
            },
            BatchSize::SmallInput,
        );
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(500)
        .measurement_time(Duration::from_secs(10))
        .noise_threshold(0.02);
    targets =
        table_create_empty,
        table_create_array,
        table_create_hash,
        table_get_set,
        table_traversal_pairs,
        table_traversal_for_each,
        table_traversal_sequence,

        function_create,
        function_call_sum,
        function_call_lua_sum,
        function_call_concat,
        function_call_lua_concat,
        function_async_call_sum,

        registry_value_create,
        registry_value_get,

        userdata_create,
        userdata_call_index,
        userdata_call_method,
        userdata_async_call_method,
}

criterion_main!(benches);

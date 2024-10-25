use std::time::Duration;

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};

use mlua::prelude::*;

fn collect_gc_twice(lua: &Lua) {
    lua.gc_collect().unwrap();
    lua.gc_collect().unwrap();
}

fn encode_json(c: &mut Criterion) {
    let lua = Lua::new();

    let encode = lua
        .create_function(|_, t: LuaValue| Ok(serde_json::to_string(&t).unwrap()))
        .unwrap();
    let table = lua
        .load(
            r#"{
        name = "Clark Kent",
        address = {
            city = "Smallville",
            state = "Kansas",
            country = "USA",
        },
        age = 22,
        parents = {"Jonathan Kent", "Martha Kent"},
        superman = true,
        interests = {"flying", "saving the world", "kryptonite"},
    }"#,
        )
        .eval::<LuaTable>()
        .unwrap();

    c.bench_function("serialize json", |b| {
        b.iter_batched(
            || collect_gc_twice(&lua),
            |_| {
                encode.call::<LuaString>(&table).unwrap();
            },
            BatchSize::SmallInput,
        );
    });
}

fn decode_json(c: &mut Criterion) {
    let lua = Lua::new();

    let decode = lua
        .create_function(|lua, s: String| {
            lua.to_value(&serde_json::from_str::<serde_json::Value>(&s).unwrap())
        })
        .unwrap();
    let json = r#"{
        "name": "Clark Kent",
        "address": {
            "city": "Smallville",
            "state": "Kansas",
            "country": "USA"
        },
        "age": 22,
        "parents": ["Jonathan Kent", "Martha Kent"],
        "superman": true,
        "interests": ["flying", "saving the world", "kryptonite"]
    }"#;

    c.bench_function("deserialize json", |b| {
        b.iter_batched(
            || collect_gc_twice(&lua),
            |_| {
                decode.call::<LuaTable>(json).unwrap();
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
        encode_json,
        decode_json,
}

criterion_main!(benches);

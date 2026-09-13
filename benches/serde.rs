use std::collections::BTreeMap;
use std::time::Duration;

use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};

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

fn sorted_map_traversal(c: &mut Criterion) {
    let lua = Lua::new();
    let mut group = c.benchmark_group("sorted map traversal");
    for size in [0, 1, 2, 3, 4, 7, 8, 64, 4096] {
        let table = lua
            .create_table_from((0..size).map(|i| (format!("key_{i:08}"), i)))
            .unwrap();
        let value = LuaValue::Table(table);
        group.bench_function(BenchmarkId::new("serialize", size), |b| {
            b.iter(|| serde_json::to_string(&value.to_serializable().sort_keys(true)).unwrap());
        });
        group.bench_function(BenchmarkId::new("from_value", size), |b| {
            b.iter(|| {
                lua.from_value_with::<BTreeMap<String, i64>>(
                    value.clone(),
                    LuaDeserializeOptions::new().sort_keys(true),
                )
                .unwrap()
            });
        });
    }
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
        sorted_map_traversal,
}

criterion_main!(benches);

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use std::time::Duration;

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
                encode.call::<_, LuaString>(&table).unwrap();
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
}

criterion_main!(benches);

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use std::time::Duration;

use mlua::prelude::*;

fn collect_gc_twice(lua: &Lua) {
    lua.gc_collect().unwrap();
    lua.gc_collect().unwrap();
}

fn serialize_json(c: &mut Criterion) {
    let lua = Lua::new();

    lua.globals()
        .set(
            "encode",
            LuaFunction::wrap(|_, t: LuaValue| Ok(serde_json::to_string(&t).unwrap())),
        )
        .unwrap();

    c.bench_function("serialize table to json [10]", |b| {
        b.iter_batched(
            || {
                collect_gc_twice(&lua);
                lua.load(
                    r#"
                local encode = encode
                return function()
                    for i = 1, 10 do
                        encode({
                            name = "Clark Kent",
                            nickname = "Superman",
                            address = {
                                city = "Metropolis",
                            },
                            age = 32,
                            superman = true,
                        })
                    end
                end
            "#,
                )
                .eval::<LuaFunction>()
                .unwrap()
            },
            |func| {
                func.call::<_, ()>(()).unwrap();
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
        serialize_json,
}

criterion_main!(benches);

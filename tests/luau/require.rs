use std::io::Result as IoResult;
use std::result::Result as StdResult;

use mlua::{Error, IntoLua, Lua, MultiValue, NavigateError, Require, Result, TextRequirer, Value};

fn run_require(lua: &Lua, path: impl IntoLua) -> Result<Value> {
    lua.load(r#"return require(...)"#).call(path)
}

fn run_require_pcall(lua: &Lua, path: impl IntoLua) -> Result<MultiValue> {
    lua.load(r#"return pcall(require, ...)"#).call(path)
}

#[track_caller]
fn get_str(value: &Value, key: impl IntoLua) -> String {
    value.as_table().unwrap().get::<String>(key).unwrap()
}

#[test]
fn test_require_errors() {
    let lua = Lua::new();

    // RequireAbsolutePath
    let res = run_require(&lua, "/an/absolute/path");
    assert!(res.is_err());
    assert!(
        (res.unwrap_err().to_string()).contains("require path must start with a valid prefix: ./, ../, or @")
    );

    // RequireUnprefixedPath
    let res = run_require(&lua, "an/unprefixed/path");
    assert!(res.is_err());
    assert!(
        (res.unwrap_err().to_string()).contains("require path must start with a valid prefix: ./, ../, or @")
    );

    // Pass non-string to require
    let res = run_require(&lua, true);
    assert!(res.is_err());
    assert!((res.unwrap_err().to_string())
        .contains("bad argument #1 to 'require' (string expected, got boolean)"));

    // Require from loadstring
    let res = lua
        .load(r#"return loadstring("require('./a/relative/path')")()"#)
        .eval::<Value>();
    assert!(res.is_err());
    assert!((res.unwrap_err().to_string()).contains("require is not supported in this context"));

    // Test throwing mlua::Error
    struct MyRequire(TextRequirer);

    impl Require for MyRequire {
        fn is_require_allowed(&self, chunk_name: &str) -> bool {
            self.0.is_require_allowed(chunk_name)
        }

        fn reset(&mut self, _chunk_name: &str) -> StdResult<(), NavigateError> {
            Err(Error::runtime("test error"))?
        }

        fn jump_to_alias(&mut self, path: &str) -> StdResult<(), NavigateError> {
            self.0.jump_to_alias(path)
        }

        fn to_parent(&mut self) -> StdResult<(), NavigateError> {
            self.0.to_parent()
        }

        fn to_child(&mut self, name: &str) -> StdResult<(), NavigateError> {
            self.0.to_child(name)
        }

        fn has_module(&self) -> bool {
            self.0.has_module()
        }

        fn cache_key(&self) -> String {
            self.0.cache_key()
        }

        fn has_config(&self) -> bool {
            self.0.has_config()
        }

        fn config(&self) -> IoResult<Vec<u8>> {
            self.0.config()
        }

        fn loader(&self, lua: &Lua) -> Result<mlua::Function> {
            self.0.loader(lua)
        }
    }

    let require = lua
        .create_require_function(MyRequire(TextRequirer::new()))
        .unwrap();
    lua.globals().set("require", require).unwrap();
    let res = lua.load(r#"return require('./a/relative/path')"#).exec();
    assert!((res.unwrap_err().to_string()).contains("test error"));
}

#[test]
fn test_require_without_config() {
    let lua = Lua::new();

    // RequireSimpleRelativePath
    let res = run_require(&lua, "./tests/luau/require/without_config/dependency").unwrap();
    assert_eq!("result from dependency", get_str(&res, 1));

    // RequireSimpleRelativePathWithinPcall
    let res = run_require_pcall(&lua, "./tests/luau/require/without_config/dependency").unwrap();
    assert!(res[0].as_boolean().unwrap());
    assert_eq!("result from dependency", get_str(&res[1], 1));

    // RequireRelativeToRequiringFile
    let res = run_require(&lua, "./tests/luau/require/without_config/module").unwrap();
    assert_eq!("result from dependency", get_str(&res, 1));
    assert_eq!("required into module", get_str(&res, 2));

    // RequireLua
    let res = run_require(&lua, "./tests/luau/require/without_config/lua_dependency").unwrap();
    assert_eq!("result from lua_dependency", get_str(&res, 1));

    // RequireInitLuau
    let res = run_require(&lua, "./tests/luau/require/without_config/luau").unwrap();
    assert_eq!("result from init.luau", get_str(&res, 1));

    // RequireInitLua
    let res = run_require(&lua, "./tests/luau/require/without_config/lua").unwrap();
    assert_eq!("result from init.lua", get_str(&res, 1));

    // RequireSubmoduleUsingSelfIndirectly
    let res = run_require(&lua, "./tests/luau/require/without_config/nested_module_requirer").unwrap();
    assert_eq!("result from submodule", get_str(&res, 1));

    // RequireSubmoduleUsingSelfDirectly
    let res = run_require(&lua, "./tests/luau/require/without_config/nested").unwrap();
    assert_eq!("result from submodule", get_str(&res, 1));

    // CannotRequireInitLuauDirectly
    let res = run_require(&lua, "./tests/luau/require/without_config/nested/init");
    assert!(res.is_err());
    assert!((res.unwrap_err().to_string()).contains("could not resolve child component \"init\""));

    // RequireNestedInits
    let res = run_require(&lua, "./tests/luau/require/without_config/nested_inits_requirer").unwrap();
    assert_eq!("result from nested_inits/init", get_str(&res, 1));
    assert_eq!("required into module", get_str(&res, 2));

    // RequireWithFileAmbiguity
    let res = run_require(
        &lua,
        "./tests/luau/require/without_config/ambiguous_file_requirer",
    );
    assert!(res.is_err());
    assert!((res.unwrap_err().to_string())
        .contains("could not resolve child component \"dependency\" (ambiguous)"));

    // RequireWithDirectoryAmbiguity
    let res = run_require(
        &lua,
        "./tests/luau/require/without_config/ambiguous_directory_requirer",
    );
    assert!(res.is_err());
    assert!((res.unwrap_err().to_string())
        .contains("could not resolve child component \"dependency\" (ambiguous)"));

    // CheckCachedResult
    let res = run_require(&lua, "./tests/luau/require/without_config/validate_cache").unwrap();
    assert!(res.is_table());
}

#[test]
fn test_require_with_config() {
    let lua = Lua::new();

    // RequirePathWithAlias
    let res = run_require(&lua, "./tests/luau/require/with_config/src/alias_requirer").unwrap();
    assert_eq!("result from dependency", get_str(&res, 1));

    // RequirePathWithParentAlias
    let res = run_require(&lua, "./tests/luau/require/with_config/src/parent_alias_requirer").unwrap();
    assert_eq!("result from other_dependency", get_str(&res, 1));

    // RequirePathWithAliasPointingToDirectory
    let res = run_require(
        &lua,
        "./tests/luau/require/with_config/src/directory_alias_requirer",
    )
    .unwrap();
    assert_eq!("result from subdirectory_dependency", get_str(&res, 1));

    // RequireAliasThatDoesNotExist
    let res = run_require(&lua, "@this.alias.does.not.exist");
    assert!(res.is_err());
    assert!((res.unwrap_err().to_string()).contains("@this.alias.does.not.exist is not a valid alias"));

    // IllegalAlias
    let res = run_require(&lua, "@");
    assert!(res.is_err());
    assert!((res.unwrap_err().to_string()).contains("@ is not a valid alias"));
}

#[cfg(all(feature = "async", not(windows)))]
#[tokio::test]
async fn test_async_require() -> Result<()> {
    let lua = Lua::new();

    let temp_dir = tempfile::tempdir().unwrap();
    let temp_path = temp_dir.path().join("async_chunk.luau");
    std::fs::write(
        &temp_path,
        r#"
        sleep_ms(10)
        return "result_after_async_sleep"
    "#,
    )
    .unwrap();

    lua.globals().set(
        "sleep_ms",
        lua.create_async_function(|_, ms: u64| async move {
            tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
            Ok(())
        })?,
    )?;
    lua.globals().set("tmp_dir", temp_dir.path().to_str().unwrap())?;
    lua.globals().set(
        "curr_dir_components",
        std::env::current_dir().unwrap().components().count(),
    )?;

    lua.load(
        r#"
        local path_to_root = string.rep("/..", curr_dir_components - 1)
        local result = require(`.{path_to_root}{tmp_dir}/async_chunk`)
        assert(result == "result_after_async_sleep")
        "#,
    )
    .exec_async()
    .await
}

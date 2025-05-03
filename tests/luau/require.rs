use mlua::{IntoLua, Lua, Result, Value};

fn run_require(lua: &Lua, path: &str) -> Result<Value> {
    lua.load(r#"return require(...)"#).call(path)
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
}

#[test]
fn test_require_without_config() {
    let lua = Lua::new();

    // RequireSimpleRelativePath
    let res = run_require(&lua, "./require/without_config/dependency").unwrap();
    assert_eq!("result from dependency", get_str(&res, 1));

    // RequireRelativeToRequiringFile
    let res = run_require(&lua, "./require/without_config/module").unwrap();
    assert_eq!("result from dependency", get_str(&res, 1));
    assert_eq!("required into module", get_str(&res, 2));

    // RequireLua
    let res = run_require(&lua, "./require/without_config/lua_dependency").unwrap();
    assert_eq!("result from lua_dependency", get_str(&res, 1));

    // RequireInitLuau
    let res = run_require(&lua, "./require/without_config/luau").unwrap();
    assert_eq!("result from init.luau", get_str(&res, 1));

    // RequireInitLua
    let res = run_require(&lua, "./require/without_config/lua").unwrap();
    assert_eq!("result from init.lua", get_str(&res, 1));

    // RequireSubmoduleUsingSelf
    let res = run_require(&lua, "./require/without_config/nested_module_requirer").unwrap();
    assert_eq!("result from submodule", get_str(&res, 1));

    // RequireWithFileAmbiguity
    let res = run_require(&lua, "./require/without_config/ambiguous_file_requirer");
    assert!(res.is_err());
    assert!((res.unwrap_err().to_string())
        .contains("could not resolve child component \"dependency\" (ambiguous)"));

    // RequireWithDirectoryAmbiguity
    let res = run_require(&lua, "./require/without_config/ambiguous_directory_requirer");
    assert!(res.is_err());
    assert!((res.unwrap_err().to_string())
        .contains("could not resolve child component \"dependency\" (ambiguous)"));

    // CheckCachedResult
    let res = run_require(&lua, "./require/without_config/validate_cache").unwrap();
    assert!(res.is_table());
}

#[test]
fn test_require_with_config() {
    let lua = Lua::new();

    // RequirePathWithAlias
    let res = run_require(&lua, "./require/with_config/src/alias_requirer").unwrap();
    assert_eq!("result from dependency", get_str(&res, 1));

    // RequirePathWithParentAlias
    let res = run_require(&lua, "./require/with_config/src/parent_alias_requirer").unwrap();
    assert_eq!("result from other_dependency", get_str(&res, 1));

    // RequirePathWithAliasPointingToDirectory
    let res = run_require(&lua, "./require/with_config/src/directory_alias_requirer").unwrap();
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

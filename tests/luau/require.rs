use mlua::{IntoLua, Lua, Result, Value, NavigateError, Error, prelude::LuaRequire};
use std::io::Result as IoResult;
use std::result::Result as StdResult;
use std::{env, fs};
use std::path::{Component, Path, PathBuf};
use std::cell::RefCell;
use std::collections::VecDeque;

fn run_require(lua: &Lua, path: impl IntoLua) -> Result<Value> {
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

    // Pass non-string to require
    let res = run_require(&lua, true);
    assert!(res.is_err());
    assert!((res.unwrap_err().to_string())
        .contains("bad argument #1 to 'require' (string expected, got boolean)"));
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

#[cfg(feature = "async")]
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

    lua.load(
        r#"
        local result = require("./async_chunk")
        assert(result == "result_after_async_sleep")
        "#,
    )
    .set_name(format!("@{}", temp_dir.path().join("require.rs").display()))
    .exec_async()
    .await
}

#[test]
fn test_require_custom_error() {
    let lua = Lua::new();
    lua.globals().set("require", lua.create_require_function(TextRequirer::new(true)).unwrap()).unwrap();

    let res = run_require(&lua, "@failed/failure");
    assert!(res.is_err());
    println!("{}", res.clone().unwrap_err().to_string());
    assert!((res.unwrap_err().to_string()).contains("custom error"));

    // ensure repeat calls do not lead to error
    let res = run_require(&lua, "@failed/failure");
    assert!(res.is_err());
    println!("{}", res.clone().unwrap_err().to_string());
    assert!((res.unwrap_err().to_string()).contains("custom error"));

    // Ensure valid stack after end of tests
    let stack_count: i32 = unsafe {
      	lua.exec_raw((), |state| {
		let n = mlua::ffi::lua_gettop(state);
		mlua::ffi::lua_pushinteger(state, n.into());
	}).unwrap()
    };

    assert_eq!(stack_count, 0);
}

/// Simple test require trait to test custom errors
#[derive(Default)]
struct TextRequirer {
    abs_path: RefCell<PathBuf>,
    rel_path: RefCell<PathBuf>,
    module_path: RefCell<PathBuf>,
    error_on_reset: bool,
}

impl TextRequirer {
    pub fn new(error_on_reset: bool) -> Self {
        Self {
		error_on_reset,
		..Default::default()
	}
    }

    fn normalize_chunk_name(chunk_name: &str) -> &str {
        if let Some((path, line)) = chunk_name.split_once(':') {
            if line.parse::<u32>().is_ok() {
                return path;
            }
        }
        chunk_name
    }

    // Normalizes the path by removing unnecessary components
    fn normalize_path(path: &Path) -> PathBuf {
        let mut components = VecDeque::new();

        for comp in path.components() {
            match comp {
                Component::Prefix(..) | Component::RootDir => {
                    components.push_back(comp);
                }
                Component::CurDir => {}
                Component::ParentDir => {
                    if matches!(components.back(), None | Some(Component::ParentDir)) {
                        components.push_back(Component::ParentDir);
                    } else if matches!(components.back(), Some(Component::Normal(..))) {
                        components.pop_back();
                    }
                }
                Component::Normal(..) => components.push_back(comp),
            }
        }

        if matches!(components.front(), None | Some(Component::Normal(..))) {
            components.push_front(Component::CurDir);
        }

        // Join the components back together
        components.into_iter().collect()
    }

    fn find_module_path(path: &Path) -> StdResult<PathBuf, NavigateError> {
        let mut found_path = None;

        let current_ext = (path.extension().and_then(|s| s.to_str()))
            .map(|s| format!("{s}."))
            .unwrap_or_default();
        for ext in ["luau", "lua"] {
            let candidate = path.with_extension(format!("{current_ext}{ext}"));
            if candidate.is_file() {
                if found_path.is_some() {
                    return Err(NavigateError::Ambiguous);
                }
                found_path = Some(candidate);
            }
        }
        if path.is_dir() {
            if found_path.is_some() {
                return Err(NavigateError::Ambiguous);
            }

            for component in ["init.luau", "init.lua"] {
                let candidate = path.join(component);
                if candidate.is_file() {
                    if found_path.is_some() {
                        return Err(NavigateError::Ambiguous);
                    }
                    found_path = Some(candidate);
                }
            }

            if found_path.is_none() {
                found_path = Some(PathBuf::new());
            }
        }

        found_path.ok_or(NavigateError::NotFound)
    }
}

impl LuaRequire for TextRequirer {
    fn is_require_allowed(&self, chunk_name: &str) -> bool {
        chunk_name.starts_with('@')
    }

    fn reset(&self, chunk_name: &str) -> StdResult<(), NavigateError> {
        if self.error_on_reset {
		return Err(NavigateError::Error(Error::runtime("custom error".to_string())));
	}

        if !chunk_name.starts_with('@') {
            return Err(NavigateError::NotFound);
        }
        let chunk_name = &Self::normalize_chunk_name(chunk_name)[1..];
        let path = Self::normalize_path(chunk_name.as_ref());

        if path.extension() == Some("rs".as_ref()) {
            let cwd = match env::current_dir() {
                Ok(cwd) => cwd,
                Err(_) => return Err(NavigateError::NotFound),
            };
            self.abs_path.replace(Self::normalize_path(&cwd.join(&path)));
            self.rel_path.replace(path);
            self.module_path.replace(PathBuf::new());

            return Ok(());
        }

        if path.is_absolute() {
            let module_path = Self::find_module_path(&path)?;
            self.abs_path.replace(path.clone());
            self.rel_path.replace(path);
            self.module_path.replace(module_path);
        } else {
            // Relative path
            let cwd = match env::current_dir() {
                Ok(cwd) => cwd,
                Err(_) => return Err(NavigateError::NotFound),
            };
            let abs_path = cwd.join(&path);
            let module_path = Self::find_module_path(&abs_path)?;
            self.abs_path.replace(Self::normalize_path(&abs_path));
            self.rel_path.replace(path);
            self.module_path.replace(module_path);
        }

        Ok(())
    }

    fn jump_to_alias(&self, path: &str) -> StdResult<(), NavigateError> {
        let path = Self::normalize_path(path.as_ref());
        let module_path = Self::find_module_path(&path)?;

        self.abs_path.replace(path.clone());
        self.rel_path.replace(path);
        self.module_path.replace(module_path);

        Ok(())
    }

    fn to_parent(&self) -> StdResult<(), NavigateError> {
        let mut abs_path = self.abs_path.borrow().clone();
        if !abs_path.pop() {
            return Err(NavigateError::NotFound);
        }
        let mut rel_parent = self.rel_path.borrow().clone();
        rel_parent.pop();
        let module_path = Self::find_module_path(&abs_path)?;

        self.abs_path.replace(abs_path);
        self.rel_path.replace(Self::normalize_path(&rel_parent));
        self.module_path.replace(module_path);

        Ok(())
    }

    fn to_child(&self, name: &str) -> StdResult<(), NavigateError> {
        let abs_path = self.abs_path.borrow().join(name);
        let rel_path = self.rel_path.borrow().join(name);
        let module_path = Self::find_module_path(&abs_path)?;

        self.abs_path.replace(abs_path);
        self.rel_path.replace(rel_path);
        self.module_path.replace(module_path);

        Ok(())
    }

    fn is_module_present(&self) -> bool {
        self.module_path.borrow().is_file()
    }

    fn contents(&self) -> IoResult<Vec<u8>> {
        fs::read(&*self.module_path.borrow())
    }

    fn chunk_name(&self) -> String {
        format!("@{}", self.rel_path.borrow().display())
    }

    fn cache_key(&self) -> Vec<u8> {
        self.module_path.borrow().display().to_string().into_bytes()
    }

    fn is_config_present(&self) -> bool {
        self.abs_path.borrow().join(".luaurc").is_file()
    }

    fn config(&self) -> IoResult<Vec<u8>> {
        fs::read(self.abs_path.borrow().join(".luaurc"))
    }
}


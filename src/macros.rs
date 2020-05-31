macro_rules! bug_msg {
    ($arg:expr) => {
        concat!(
            "mlua internal error: ",
            $arg,
            " (this is a bug, please file an issue)"
        )
    };
}

macro_rules! cstr {
    ($s:expr) => {
        concat!($s, "\0") as *const str as *const [::std::os::raw::c_char]
            as *const ::std::os::raw::c_char
    };
}

macro_rules! mlua_panic {
    ($msg:expr) => {
        panic!(bug_msg!($msg));
    };

    ($msg:expr,) => {
        mlua_panic!($msg);
    };

    ($msg:expr, $($arg:expr),+) => {
        panic!(bug_msg!($msg), $($arg),+);
    };

    ($msg:expr, $($arg:expr),+,) => {
        mlua_panic!($msg, $($arg),+);
    };
}

macro_rules! mlua_assert {
    ($cond:expr, $msg:expr) => {
        assert!($cond, bug_msg!($msg));
    };

    ($cond:expr, $msg:expr,) => {
        mlua_assert!($cond, $msg);
    };

    ($cond:expr, $msg:expr, $($arg:expr),+) => {
        assert!($cond, bug_msg!($msg), $($arg),+);
    };

    ($cond:expr, $msg:expr, $($arg:expr),+,) => {
        mlua_assert!($cond, $msg, $($arg),+);
    };
}

macro_rules! mlua_debug_assert {
    ($cond:expr, $msg:expr) => {
        debug_assert!($cond, bug_msg!($msg));
    };

    ($cond:expr, $msg:expr,) => {
        mlua_debug_assert!($cond, $msg);
    };

    ($cond:expr, $msg:expr, $($arg:expr),+) => {
        debug_assert!($cond, bug_msg!($msg), $($arg),+);
    };

    ($cond:expr, $msg:expr, $($arg:expr),+,) => {
        mlua_debug_assert!($cond, $msg, $($arg),+);
    };
}

macro_rules! mlua_expect {
    ($res:expr, $msg:expr) => {
        $res.expect(bug_msg!($msg))
    };

    ($res:expr, $msg:expr,) => {
        mlua_expect!($res, $msg)
    };
}

#[cfg(feature = "module")]
#[doc(hidden)]
#[macro_export]
macro_rules! require_module_feature {
    () => {};
}

#[cfg(not(feature = "module"))]
#[doc(hidden)]
#[macro_export]
macro_rules! require_module_feature {
    () => {
        compile_error!("Feature `module` must be enabled in the `mlua` crate");
    };
}

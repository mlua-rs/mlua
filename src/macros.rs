macro_rules! bug_msg {
    ($arg:expr) => {
        concat!(
            "rlua internal error: ",
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

macro_rules! rlua_panic {
    ($msg:expr) => {
        panic!(bug_msg!($msg));
    };

    ($msg:expr,) => {
        rlua_panic!($msg);
    };

    ($msg:expr, $($arg:expr),+) => {
        panic!(bug_msg!($msg), $($arg),+);
    };

    ($msg:expr, $($arg:expr),+,) => {
        rlua_panic!($msg, $($arg),+);
    };
}

macro_rules! rlua_assert {
    ($cond:expr, $msg:expr) => {
        assert!($cond, bug_msg!($msg));
    };

    ($cond:expr, $msg:expr,) => {
        rlua_assert!($cond, $msg);
    };

    ($cond:expr, $msg:expr, $($arg:expr),+) => {
        assert!($cond, bug_msg!($msg), $($arg),+);
    };

    ($cond:expr, $msg:expr, $($arg:expr),+,) => {
        rlua_assert!($cond, $msg, $($arg),+);
    };
}

macro_rules! rlua_debug_assert {
    ($cond:expr, $msg:expr) => {
        debug_assert!($cond, bug_msg!($msg));
    };

    ($cond:expr, $msg:expr,) => {
        rlua_debug_assert!($cond, $msg);
    };

    ($cond:expr, $msg:expr, $($arg:expr),+) => {
        debug_assert!($cond, bug_msg!($msg), $($arg),+);
    };

    ($cond:expr, $msg:expr, $($arg:expr),+,) => {
        rlua_debug_assert!($cond, $msg, $($arg),+);
    };
}

macro_rules! rlua_expect {
    ($res:expr, $msg:expr) => {
        $res.expect(bug_msg!($msg))
    };

    ($res:expr, $msg:expr,) => {
        rlua_expect!($res, $msg)
    };
}

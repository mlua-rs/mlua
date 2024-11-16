use std::cell::Cell;
use std::marker::PhantomData;
use std::os::raw::c_int;

use super::UserDataStorage;
use crate::util::{get_userdata, take_userdata};

// This is a trick to check if a type is `Sync` or not.
// It uses leaked specialization feature from stdlib.
struct IsSync<'a, T> {
    is_sync: &'a Cell<bool>,
    _marker: PhantomData<T>,
}

impl<T> Clone for IsSync<'_, T> {
    fn clone(&self) -> Self {
        self.is_sync.set(false);
        IsSync {
            is_sync: self.is_sync,
            _marker: PhantomData,
        }
    }
}

impl<T: Sync> Copy for IsSync<'_, T> {}

pub(crate) fn is_sync<T>() -> bool {
    let is_sync = Cell::new(true);
    let _ = [IsSync::<T> {
        is_sync: &is_sync,
        _marker: PhantomData,
    }]
    .clone();
    is_sync.get()
}

pub(super) unsafe extern "C-unwind" fn userdata_destructor<T>(state: *mut ffi::lua_State) -> c_int {
    let ud = get_userdata::<UserDataStorage<T>>(state, -1);
    if !(*ud).is_borrowed() {
        take_userdata::<UserDataStorage<T>>(state);
        ffi::lua_pushboolean(state, 1);
    } else {
        ffi::lua_pushboolean(state, 0);
    }
    1
}

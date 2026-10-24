use mlua::{Error, Lua};

#[cfg(target_os = "unknown")]
mod allocator {
    use std::alloc::{GlobalAlloc, Layout};
    use std::ffi::c_void;

    // Rust and WASI libc must share an allocator
    struct WasiAllocator;

    unsafe extern "C" {
        fn posix_memalign(ptr: *mut *mut c_void, align: usize, size: usize) -> i32;
        fn free(ptr: *mut c_void);
    }

    unsafe impl GlobalAlloc for WasiAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let mut ptr = std::ptr::null_mut();
            if unsafe { posix_memalign(&mut ptr, layout.align().max(size_of::<usize>()), layout.size()) } != 0
            {
                return std::ptr::null_mut();
            }
            ptr.cast()
        }

        unsafe fn dealloc(&self, ptr: *mut u8, _: Layout) {
            unsafe { free(ptr.cast()) };
        }
    }

    #[global_allocator]
    static ALLOCATOR: WasiAllocator = WasiAllocator;
}

#[cfg(target_os = "unknown")]
#[unsafe(no_mangle)]
pub extern "C" fn run_tests() {
    test_eval();
    test_errors();
    test_callbacks();
    test_coroutines();
}

#[cfg_attr(target_os = "wasi", test)]
fn test_eval() {
    let lua = Lua::new();
    assert_eq!(lua.load("return 6 * 7").eval::<i32>().unwrap(), 42);
}

#[cfg_attr(target_os = "wasi", test)]
fn test_errors() {
    let lua = Lua::new();
    assert!(matches!(
        lua.load("local =").exec(),
        Err(Error::SyntaxError { .. })
    ));
    assert!(matches!(
        lua.load("error('test error')").exec(),
        Err(Error::RuntimeError(_))
    ));
    assert!(
        !lua.load("return pcall(function() error('test error') end)")
            .eval::<bool>()
            .unwrap()
    );
    assert_eq!(lua.load("return 42").eval::<i32>().unwrap(), 42);
}

#[cfg_attr(target_os = "wasi", test)]
fn test_callbacks() {
    let lua = Lua::new();
    let double = lua.create_function(|_, n: i32| Ok(n * 2)).unwrap();
    lua.globals().set("double", double).unwrap();
    assert_eq!(lua.load("return double(21)").eval::<i32>().unwrap(), 42);
    let fail = lua
        .create_function(|_, ()| Err::<(), _>(Error::runtime("callback error")))
        .unwrap();
    lua.globals().set("fail", fail).unwrap();
    assert!(matches!(
        lua.load("fail()").exec(),
        Err(Error::CallbackError { .. })
    ));
    assert!(!lua.load("return pcall(fail)").eval::<bool>().unwrap());
}

#[cfg_attr(target_os = "wasi", test)]
fn test_coroutines() {
    let lua = Lua::new();
    let function = lua
        .load("local n = ...; coroutine.yield(n); return n * 2")
        .into_function()
        .unwrap();
    let thread = lua.create_thread(function).unwrap();
    assert_eq!(thread.resume::<i32>(21).unwrap(), 21);
    assert_eq!(thread.resume::<i32>(()).unwrap(), 42);
}

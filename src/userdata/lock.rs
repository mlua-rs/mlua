pub(crate) trait UserDataLock {
    const INIT: Self;

    fn try_lock_shared(&self) -> bool;
    fn try_lock_exclusive(&self) -> bool;

    unsafe fn unlock_shared(&self);
    unsafe fn unlock_exclusive(&self);
}

pub(crate) use lock_impl::RawLock;

#[cfg(not(feature = "send"))]
#[cfg(not(tarpaulin_include))]
mod lock_impl {
    use std::cell::Cell;

    // Positive values represent the number of read references.
    // Negative values represent the number of write references (only one allowed).
    pub(crate) type RawLock = Cell<isize>;

    const UNUSED: isize = 0;

    impl super::UserDataLock for RawLock {
        #[allow(clippy::declare_interior_mutable_const)]
        const INIT: Self = Cell::new(UNUSED);

        #[inline(always)]
        fn try_lock_shared(&self) -> bool {
            let flag = self.get().wrapping_add(1);
            if flag <= UNUSED {
                return false;
            }
            self.set(flag);
            true
        }

        #[inline(always)]
        fn try_lock_exclusive(&self) -> bool {
            let flag = self.get();
            if flag != UNUSED {
                return false;
            }
            self.set(UNUSED - 1);
            true
        }

        #[inline(always)]
        unsafe fn unlock_shared(&self) {
            let flag = self.get();
            debug_assert!(flag > UNUSED);
            self.set(flag - 1);
        }

        #[inline(always)]
        unsafe fn unlock_exclusive(&self) {
            let flag = self.get();
            debug_assert!(flag < UNUSED);
            self.set(flag + 1);
        }
    }
}

#[cfg(feature = "send")]
mod lock_impl {
    use parking_lot::lock_api::RawRwLock;

    pub(crate) type RawLock = parking_lot::RawRwLock;

    impl super::UserDataLock for RawLock {
        #[allow(clippy::declare_interior_mutable_const)]
        const INIT: Self = <Self as parking_lot::lock_api::RawRwLock>::INIT;

        #[inline(always)]
        fn try_lock_shared(&self) -> bool {
            RawRwLock::try_lock_shared(self)
        }

        #[inline(always)]
        fn try_lock_exclusive(&self) -> bool {
            RawRwLock::try_lock_exclusive(self)
        }

        #[inline(always)]
        unsafe fn unlock_shared(&self) {
            RawRwLock::unlock_shared(self)
        }

        #[inline(always)]
        unsafe fn unlock_exclusive(&self) {
            RawRwLock::unlock_exclusive(self)
        }
    }
}

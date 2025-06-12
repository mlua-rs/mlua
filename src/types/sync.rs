#[cfg(feature = "send")]
mod inner {
    use parking_lot::{RawMutex, RawThreadId};
    use std::sync::{Arc, Weak};

    pub(crate) type XRc<T> = Arc<T>;
    pub(crate) type XWeak<T> = Weak<T>;

    pub(crate) type ReentrantMutex<T> = parking_lot::ReentrantMutex<T>;

    pub(crate) type ReentrantMutexGuard<'a, T> = parking_lot::ReentrantMutexGuard<'a, T>;

    pub(crate) type ArcReentrantMutexGuard<T> =
        parking_lot::lock_api::ArcReentrantMutexGuard<RawMutex, RawThreadId, T>;
}

#[cfg(not(feature = "send"))]
mod inner {
    use std::ops::Deref;
    use std::rc::{Rc, Weak};

    pub(crate) type XRc<T> = Rc<T>;
    pub(crate) type XWeak<T> = Weak<T>;

    pub(crate) struct ReentrantMutex<T>(T);

    impl<T> ReentrantMutex<T> {
        #[inline(always)]
        pub(crate) fn new(val: T) -> Self {
            ReentrantMutex(val)
        }

        #[inline(always)]
        pub(crate) fn lock(&self) -> ReentrantMutexGuard<'_, T> {
            ReentrantMutexGuard(&self.0)
        }

        #[inline(always)]
        pub(crate) fn lock_arc(self: &XRc<Self>) -> ArcReentrantMutexGuard<T> {
            ArcReentrantMutexGuard(Rc::clone(self))
        }

        #[inline(always)]
        pub(crate) fn into_lock_arc(self: XRc<Self>) -> ArcReentrantMutexGuard<T> {
            ArcReentrantMutexGuard(self)
        }

        #[inline(always)]
        pub(crate) fn data_ptr(&self) -> *const T {
            &self.0 as *const _
        }
    }

    pub(crate) struct ReentrantMutexGuard<'a, T>(&'a T);

    impl<T> Deref for ReentrantMutexGuard<'_, T> {
        type Target = T;

        #[inline(always)]
        fn deref(&self) -> &Self::Target {
            self.0
        }
    }

    pub(crate) struct ArcReentrantMutexGuard<T>(XRc<ReentrantMutex<T>>);

    impl<T> Deref for ArcReentrantMutexGuard<T> {
        type Target = T;

        #[inline(always)]
        fn deref(&self) -> &Self::Target {
            &self.0 .0
        }
    }
}

pub(crate) use inner::{ArcReentrantMutexGuard, ReentrantMutex, ReentrantMutexGuard, XRc, XWeak};

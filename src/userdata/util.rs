use std::cell::Cell;
use std::marker::PhantomData;

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

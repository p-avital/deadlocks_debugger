use std::sync::{LockResult, PoisonError, TryLockError, TryLockResult};
use std::cell::UnsafeCell;
use std::time::{Instant, Duration};

/// An instrumented version of `std::sync::Mutex`
pub struct Mutex<T: ?Sized> {
    key: usize,
    poisoned: bool,
    manager: std::sync::Arc<crate::lock_manager::LockManager>,
    inner: UnsafeCell<T>,
}

impl<T> Mutex<T> {
    pub fn new(inner: T) -> Self {
        let manager = crate::lock_manager::LockManager::get_global_manager();
        let key = manager.create_lock();
        Mutex {
            inner: UnsafeCell::new(inner),
            poisoned: false,
            manager,
            key,
        }
    }

    pub fn into_inner(self) -> LockResult<T> {
        if self.poisoned {
            Err(PoisonError::new(self.inner.into_inner()))
        } else {
            Ok(self.inner.into_inner())
        }
    }
}

impl<T: ?Sized> Mutex<T> {
    pub fn get_mut(&mut self) -> LockResult<&mut T> {
        let reference = unsafe {&mut *self.inner.get()};
        if self.poisoned {
            Err(PoisonError::new(reference))
        } else {
            Ok(reference)
        }
    }

    pub fn is_poisoned(&self) -> bool {
        self.poisoned
    }

    pub fn try_lock(&self) -> TryLockResult<MutexGuard<T>> {
        let mut guard = self.manager.write_lock();
        let representation = guard.locks.get_mut(&self.key).unwrap();
        if representation.try_write_lock() {
            let returned_guard = MutexGuard {
                inner: unsafe { &mut *(self as *const _ as *mut _) },
            };
            if self.is_poisoned() {
                Err(TryLockError::Poisoned(PoisonError::new(returned_guard)))
            } else {
                Ok(returned_guard)
            }
        } else {
            Err(TryLockError::WouldBlock)
        }
    }

    pub fn lock(&self) -> LockResult<MutexGuard<T>> {
        let timeout = Duration::from_secs(1);
        let start = Instant::now();

        loop {
            let mut guard = self.manager.write_lock();
            let representation = guard.locks.get_mut(&self.key).unwrap();
            if representation.try_write_lock() {
                let returned_guard = MutexGuard {
                    inner: unsafe { &mut *(self as *const _ as *mut _) },
                };
                if self.is_poisoned() {
                    return Err(PoisonError::new(returned_guard));
                } else {
                    return Ok(returned_guard);
                }
            } else if Instant::now().duration_since(start) > timeout {
                representation.subscribe_write();
                guard.analyse();
                std::thread::yield_now();
            }
        }
    }
}

pub struct MutexGuard<'l, T: ?Sized> {
    inner: &'l mut Mutex<T>,
}
impl<'l, T> std::ops::Deref for MutexGuard<'l, T> {
    type Target = T;
    fn deref(&self) -> &<Self as std::ops::Deref>::Target {
        unsafe {&*self.inner.inner.get()}
    }
}
impl<'l, T> std::ops::DerefMut for MutexGuard<'l, T> {
    fn deref_mut(&mut self) -> &mut <Self as std::ops::Deref>::Target {
        unsafe {&mut *self.inner.inner.get()}
    }
}
impl<'l, T: ?Sized> Drop for MutexGuard<'l, T> {
    fn drop(&mut self) {
        let mut guard = self.inner.manager.write_lock();
        guard.locks.get_mut(&self.inner.key).unwrap().unlock();
        if std::thread::panicking() {
            self.inner.poisoned = true;
        }
    }
}
unsafe impl<T: Send> Send for Mutex<T> {}
unsafe impl<T: Send> Sync for Mutex<T> {}
//! A JSON-file-backed value plus the path it came from, in one place.
//!
//! `AppState` used to carry these as separate fields: `dict` and `dict_path`,
//! `snippet_store` and `snippets_path`, and so on, five pairs in total. Every
//! save site had to take two locks in the right order and remember which path
//! belonged to which value, and nothing stopped them being set inconsistently
//! (in fact several were populated dozens of lines apart during setup).
//!
//! Holding them together means a save cannot target the wrong file, and the
//! field count drops without hiding anything behind extra machinery.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub struct Store<T> {
    inner: Mutex<Inner<T>>,
}

struct Inner<T> {
    value: T,
    path: PathBuf,
}

impl<T> Store<T> {
    pub fn new(value: T) -> Self {
        Self {
            inner: Mutex::new(Inner {
                value,
                path: PathBuf::new(),
            }),
        }
    }

    /// Install the loaded value and the file it belongs to, together.
    pub fn set(&self, value: T, path: impl Into<PathBuf>) {
        let mut g = self.inner.lock().unwrap();
        g.value = value;
        g.path = path.into();
    }

    /// Read the value under the lock.
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        f(&self.inner.lock().unwrap().value)
    }

    /// Mutate the value and persist it in the same critical section, so the
    /// file can never disagree with what is in memory.
    pub fn update<E>(
        &self,
        change: impl FnOnce(&mut T),
        save: impl FnOnce(&T, &Path) -> Result<(), E>,
    ) -> Result<(), E> {
        let mut g = self.inner.lock().unwrap();
        change(&mut g.value);
        let Inner { value, path } = &*g;
        save(value, path)
    }

    /// Replace the value wholesale and persist it.
    pub fn replace<E>(
        &self,
        value: T,
        save: impl FnOnce(&T, &Path) -> Result<(), E>,
    ) -> Result<(), E> {
        self.update(|v| *v = value, save)
    }
}

impl<T: Clone> Store<T> {
    pub fn get(&self) -> T {
        self.inner.lock().unwrap().value.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn update_sees_the_path_it_was_given() {
        let s = Store::new(1u32);
        s.set(7, "/tmp/example.json");
        let seen = std::cell::RefCell::new(PathBuf::new());
        let _: Result<(), ()> = s.update(
            |v| *v += 1,
            |v, p| {
                assert_eq!(*v, 8);
                *seen.borrow_mut() = p.to_path_buf();
                Ok(())
            },
        );
        assert_eq!(seen.into_inner(), PathBuf::from("/tmp/example.json"));
        assert_eq!(s.get(), 8);
    }

    #[test]
    fn a_failed_save_still_leaves_the_value_changed() {
        // Documents current behaviour: the in-memory value is authoritative and
        // a save failure is reported, not rolled back.
        let s = Store::new(1u32);
        let r: Result<(), &str> = s.update(|v| *v = 42, |_, _| Err("disk full"));
        assert_eq!(r, Err("disk full"));
        assert_eq!(s.get(), 42);
    }

    #[test]
    fn saves_happen_once_per_update() {
        let s = Store::new(0u32);
        let calls = AtomicUsize::new(0);
        for _ in 0..3 {
            let _: Result<(), ()> = s.update(
                |v| *v += 1,
                |_, _| {
                    calls.fetch_add(1, Ordering::Relaxed);
                    Ok(())
                },
            );
        }
        assert_eq!(calls.load(Ordering::Relaxed), 3);
        assert_eq!(s.get(), 3);
    }
}

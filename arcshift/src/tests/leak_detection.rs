//! Helpers for keeping track of allocated instance counts,
//! detecting when instances are leaked.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::sync::atomic::Ordering;
use std::sync::Mutex;

/// A little helper struct that just keeps track of the number of live
/// instances of it. This is used together with Loom to ensure there
/// are no memory leaks due to race-conditions.
pub(crate) struct InstanceSpy {
    x: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}
impl InstanceSpy {
    pub(crate) fn new(x: std::sync::Arc<std::sync::atomic::AtomicUsize>) -> InstanceSpy {
        let _temp = x.fetch_add(1, Ordering::Relaxed);
        debug_println!("++ InstanceSpy ++ {}", _temp + 1);
        InstanceSpy { x }
    }
}
impl Drop for InstanceSpy {
    fn drop(&mut self) {
        let _prev = self.x.fetch_sub(1, Ordering::Relaxed);
        debug_println!("-- InstanceSpy -- drop {}", _prev - 1);
    }
}

pub(crate) struct SpyOwner2 {
    data: std::sync::Arc<Mutex<HashSet<&'static str>>>,
}

impl SpyOwner2 {
    pub(crate) fn new() -> SpyOwner2 {
        SpyOwner2 {
            data: std::sync::Arc::new(Mutex::new(HashSet::new())),
        }
    }
    pub(crate) fn create(&self, name: &'static str) -> InstanceSpy2 {
        InstanceSpy2::new(self.data.clone(), name)
    }
    pub(crate) fn validate(&self) {
        let guard = self.data.lock().unwrap();
        if guard.len() > 0 {
            panic!("Leaked: {:?}", &*guard);
        }
    }
}


#[derive(Debug, Clone)]
pub(crate) struct InstanceSpy2 {
    x: std::sync::Arc<Mutex<HashSet<&'static str>>>,
    name: &'static str,
}

impl Hash for InstanceSpy2 {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state)
    }
}
impl PartialEq<Self> for InstanceSpy2 {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for InstanceSpy2 {}

impl InstanceSpy2 {
    fn str(&self) -> &'static str {
        self.name
    }
    fn new(
        x: std::sync::Arc<Mutex<HashSet<&'static str>>>,
        name: &'static str,
    ) -> InstanceSpy2 {
        let mut guard = x.lock().unwrap();
        guard.insert(name);
        debug_println!("++ InstanceSpy ++ {:?} (added: {})", &*guard, name);
        drop(guard);
        InstanceSpy2 { x, name }
    }
}
impl Drop for InstanceSpy2 {
    fn drop(&mut self) {
        let mut guard = self.x.lock().unwrap();
        guard.remove(self.name);
        debug_println!("-- InstanceSpy -- {:?} - removed {}", &*guard, self.name);
        //debug_println!("Drop stacktrace: {:?}", Backtrace::capture());
    }
}

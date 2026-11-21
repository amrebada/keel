//! Typed interfaces one plugin offers another, e.g. `auth-core` offers
//! `identity-provider` to `auth-oauth`. The provider lists the name under
//! `provides.interfaces`; the consumer lists the provider under
//! `requires.plugins`, so it starts after it.

use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Shared by every plugin of one host.
#[derive(Clone, Default)]
pub struct Interfaces {
    inner: Arc<RwLock<HashMap<String, Arc<dyn Any + Send + Sync>>>>,
}

impl Interfaces {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert<T: ?Sized + Send + Sync + 'static>(&self, name: &str, value: Arc<T>) {
        self.inner
            .write()
            .expect("interfaces lock")
            .insert(name.to_string(), Arc::new(value));
    }

    /// The interface `name`, if one was provided with type `T`.
    pub fn get<T: ?Sized + Send + Sync + 'static>(&self, name: &str) -> Option<Arc<T>> {
        self.inner
            .read()
            .expect("interfaces lock")
            .get(name)
            .and_then(|v| v.downcast_ref::<Arc<T>>())
            .cloned()
    }

    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .inner
            .read()
            .expect("interfaces lock")
            .keys()
            .cloned()
            .collect();
        v.sort();
        v
    }
}

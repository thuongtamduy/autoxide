//! The registry of traits that modules publish to each other (`StartContext::provide` / `require` / `later` /
//! `contribute`).

use std::any::{Any, TypeId};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use crate::error::Error;

use super::Later;

/// A contributor entry: the contributing module and the `Arc<T>` stored as `dyn Any`.
type Contribution = (&'static str, Box<dyn Any + Send + Sync>);

/// The registry of published traits, looked up by the trait's type.
#[derive(Default)]
pub(super) struct Apis {
    // Khoá là "danh tính" của kiểu `T` (`TypeId`); giá trị là một `Arc<T>` cất dưới
    // dạng `dyn Any` vì mỗi mục một kiểu khác nhau. Lấy ra thì ép kiểu ngược lại
    // có kiểm tra (`downcast_ref`) — sai kiểu trả `None` chứ không hỏng bộ nhớ.
    provided: HashMap<TypeId, (&'static str, Box<dyn Any + Send + Sync>)>,
    /// `(module, providing module)` each time a module `require`s a trait of another module: the module
    /// registry and `plans` learn which module needs which (step 35). A `Mutex` because `require` only
    /// borrows `&self`.
    required: std::sync::Mutex<BTreeSet<(&'static str, &'static str)>>,
    /// Traits that many parties contribute to (`contribute`): by type, a list of `(module, Arc<T>)` in
    /// contribution order.
    contributed: HashMap<TypeId, Vec<Contribution>>,
    /// The filling of [`Later`] handles, run once after every module has started.
    pending: Vec<PendingFill>,
}

/// One filling of a [`Later`] handle: receives the complete trait registry.
type PendingFill = Box<dyn FnOnce(&Apis) + Send + Sync>;

impl Apis {
    pub(super) fn provide<T>(&mut self, module: &'static str, api: Arc<T>) -> Result<(), Error>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        let contract = std::any::type_name::<T>();
        if let Some((owner, _)) = self.provided.get(&TypeId::of::<T>()) {
            return Err(Error::internal(format!(
                "module `{module}` provides `{contract}`, but module `{owner}` already does; \
                 a contract has exactly one provider"
            )));
        }
        self.provided.insert(TypeId::of::<T>(), (module, Box::new(api)));
        Ok(())
    }

    pub(super) fn later<T>(&mut self) -> Later<T>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        let later = Later::empty();
        let slot = later.clone();
        self.pending.push(Box::new(move |apis: &Apis| slot.fill(apis.get::<T>())));
        later
    }

    /// Fills every [`Later`] handle that was handed out (called after every module has started).
    pub(super) fn fill_later(&mut self) {
        for fill in std::mem::take(&mut self.pending) {
            fill(self);
        }
    }

    pub(super) fn contribute<T>(&mut self, module: &'static str, api: Arc<T>) -> Result<(), Error>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        let entries = self.contributed.entry(TypeId::of::<T>()).or_default();
        if entries.iter().any(|(owner, _)| *owner == module) {
            return Err(Error::internal(format!(
                "module `{module}` contributes `{}` twice; contribute each contract once",
                std::any::type_name::<T>()
            )));
        }
        entries.push((module, Box::new(api)));
        Ok(())
    }

    pub(super) fn contributions<T>(&self) -> Vec<Arc<T>>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.contributed
            .get(&TypeId::of::<T>())
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|(_, api)| api.downcast_ref::<Arc<T>>().cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Like [`contributions`](Self::contributions), with the contributing module of each.
    pub(super) fn contributions_named<T>(&self) -> Vec<(&'static str, Arc<T>)>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.contributed
            .get(&TypeId::of::<T>())
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|(module, api)| {
                        api.downcast_ref::<Arc<T>>().map(|api| (*module, Arc::clone(api)))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(super) fn get<T>(&self) -> Option<Arc<T>>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.provided
            .get(&TypeId::of::<T>())
            .and_then(|(_, api)| api.downcast_ref::<Arc<T>>())
            .cloned()
    }

    /// The module that published `T`, if any.
    pub(super) fn provider_of<T>(&self) -> Option<&'static str>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.provided.get(&TypeId::of::<T>()).map(|(module, _)| *module)
    }

    /// The `(module, providing module)` pairs recorded.
    pub(super) fn required(&self) -> BTreeSet<(&'static str, &'static str)> {
        self.required.lock().map(|required| required.clone()).unwrap_or_default()
    }

    pub(super) fn require<T>(&self, module: &'static str) -> Result<Arc<T>, Error>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        if let Some(provider) = self.provider_of::<T>()
            && provider != module
            && let Ok(mut required) = self.required.lock()
        {
            required.insert((module, provider));
        }
        self.get().ok_or_else(|| {
            Error::internal(format!(
                "module `{module}` requires `{}`, which no enabled module registered before it \
                 provides. The providing module must be registered before `{module}`, be named \
                 in `Module::depends_on` of `{module}`, and not be left out by AUTOXIDE_MODULES",
                std::any::type_name::<T>()
            ))
        })
    }
}

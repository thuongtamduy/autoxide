//! Collects, after every module has started, the per-module kernel contributions that have both an old path
//! (`Module::seed_tenant`, `Module::sample_data`, `Module::sandbox_copy`) and the new one
//! (`context.contribute::<dyn …>(…)`, D-20261128-contribute).

use std::sync::Arc;

use super::{ModuleError, Modules};
use crate::module::Module;

impl Modules {
    /// One implementation per module, in registration order: from the old function `old` or from a
    /// contribution of `T`. A module that does both is a startup error (one way per concept).
    pub(super) fn per_module<T>(
        &self,
        what: &str,
        old: impl Fn(&dyn Module) -> Option<Arc<T>>,
    ) -> Result<Vec<(&'static str, Arc<T>)>, ModuleError>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        let contributed = self.apis.contributions_named::<T>();
        let mut found = Vec::new();
        for module in &self.modules {
            let name = module.name();
            let new = contributed.iter().find(|(owner, _)| *owner == name).map(|(_, api)| api);
            match (old(module.as_ref()), new) {
                (Some(_), Some(_)) => {
                    return Err(ModuleError(format!(
                        "module `{name}` implements `Module::{what}` and also contributes `{}` in `start`: \
                         keep the contribution and remove the old function",
                        std::any::type_name::<T>()
                    )));
                }
                (Some(api), None) => found.push((name, api)),
                (None, Some(api)) => found.push((name, Arc::clone(api))),
                (None, None) => {}
            }
        }
        Ok(found)
    }
}

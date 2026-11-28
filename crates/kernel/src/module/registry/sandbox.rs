//! Fills the sandbox copy / push registry (step 38, D-232) after every module has started.

use std::sync::Arc;

use super::{ModuleError, Modules};

impl Modules {
    /// Every implementation the modules contributed for `T` (`StartContext::contribute`), in registration
    /// order - already passed through the sandbox's partner wrapper (D-232).
    pub fn contributions<T>(&self) -> Vec<Arc<T>>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.apis.contributions()
    }

    /// Collects the sandbox copies (`contribute::<dyn SandboxCopy>` or the old `Module::sandbox_copy`) and `Module::promotable` in registration order. A promotable kind must
    /// be named `<module>.<kind>` and declared once.
    pub(super) fn fill_sandbox(&self) -> Result<(), ModuleError> {
        let copies = self.per_module::<dyn crate::tenancy::sandbox::SandboxCopy>("sandbox_copy", |module| {
            module.sandbox_copy()
        })?;
        let mut promotables: Vec<(&'static str, crate::tenancy::sandbox::Promotable)> = Vec::new();
        for module in &self.modules {
            let name = module.name();
            for promotable in module.promotable() {
                let prefix = format!("{name}.");
                if !promotable.kind.starts_with(&prefix)
                    || promotables.iter().any(|(_, known)| known.kind == promotable.kind)
                {
                    return Err(ModuleError(format!(
                        "promotable kind `{}` of module `{name}` must be named `{name}.<kind>` and \
                         be declared once",
                        promotable.kind
                    )));
                }
                promotables.push((name, promotable));
            }
        }
        self.sandbox.fill(copies, promotables);
        Ok(())
    }
}

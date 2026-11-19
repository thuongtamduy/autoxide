//! Điền sổ sao / đẩy sandbox (bước 38, D-232) sau khi mọi module khởi động.

use std::sync::Arc;

use super::{ModuleError, Modules};

impl Modules {
    /// Mọi bộ cài đặt các module đã góp cho `T` (`StartContext::contribute`), theo thứ tự đăng ký —
    /// đã qua bộ bọc đối tác của sandbox (D-232).
    pub fn contributions<T>(&self) -> Vec<Arc<T>>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.apis.contributions()
    }

    /// Gom `Module::sandbox_copy` / `Module::promotable` theo thứ tự đăng ký. Loại đẩy được phải
    /// tên `<module>.<loại>` và khai một lần.
    pub(super) fn fill_sandbox(&self) -> Result<(), ModuleError> {
        let mut copies = Vec::new();
        let mut promotables: Vec<(&'static str, crate::tenancy::sandbox::Promotable)> = Vec::new();
        for module in &self.modules {
            let name = module.name();
            if let Some(copy) = module.sandbox_copy() {
                copies.push((name, copy));
            }
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

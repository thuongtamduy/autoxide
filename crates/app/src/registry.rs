//! Danh sách module của chương trình đang chạy: module chính thức (`modules.rs`) cộng module mà
//! binary thêm qua `Autoxide::module` (module ở repo riêng, D-20261121-external-modules).
//!
//! Module thêm vào giữ ở một biến toàn tiến trình, đặt đúng một lần trước khi lệnh chạy: mọi lệnh
//! đọc cấu hình qua `settings::load`, nên không lệnh nào phải nhận thêm tham số. Test trong crate
//! này không đặt biến đó và thấy đúng danh sách chính thức.

use std::sync::OnceLock;

use autoxide_kernel::module::Module;

use crate::modules;

/// Hàm tạo một module. Gọi được nhiều lần: `Module` không `Clone`, mà danh sách module được dựng
/// lại ở mỗi lần đọc cấu hình.
pub type Factory = Box<dyn Fn() -> Box<dyn Module> + Send + Sync>;

static EXTRA: OnceLock<Vec<Factory>> = OnceLock::new();

/// Ghi nhận module thêm của chương trình. Lỗi nếu đã ghi nhận trước đó với module khác: một tiến
/// trình là một chương trình.
pub fn install(extra: Vec<Factory>) -> Result<(), String> {
    let added = !extra.is_empty();
    match EXTRA.set(extra) {
        Ok(()) => Ok(()),
        // Chạy lại mà không thêm gì (vd hai lần `run_from` của binary chính thức) thì vô hại.
        Err(_) if !added && EXTRA.get().is_some_and(Vec::is_empty) => Ok(()),
        Err(_) => Err("the modules of this program were already set; \
                       run one `Autoxide` per process"
            .to_owned()),
    }
}

/// Mọi module của chương trình, theo thứ tự đăng ký: chính thức trước, module thêm sau (nên
/// module thêm `require` được mọi trait của module chính thức).
pub fn all() -> Vec<Box<dyn Module>> {
    assemble(modules::all(), EXTRA.get().map_or(&[], Vec::as_slice))
}

fn assemble(official: Vec<Box<dyn Module>>, extra: &[Factory]) -> Vec<Box<dyn Module>> {
    let mut modules = official;
    modules.extend(extra.iter().map(|factory| factory()));
    modules
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Extra;

    impl Module for Extra {
        fn name(&self) -> &'static str {
            "extra"
        }
    }

    #[test]
    fn module_them_dung_sau_module_chinh_thuc() {
        let extra: Vec<Factory> = vec![Box::new(|| Box::new(Extra))];
        let names: Vec<&str> =
            assemble(modules::all(), &extra).iter().map(|module| module.name()).collect();
        assert_eq!(names.last(), Some(&"extra"));
        assert_eq!(names.len(), modules::all().len() + 1);
        // Dựng lại lần nữa vẫn ra module mới: hàm tạo gọi được nhiều lần.
        assert_eq!(assemble(Vec::new(), &extra).len(), 1);
    }
}

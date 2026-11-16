//! Permission của module `media` (D-086).

// Theo nút (bước 34, D-208): người của một cửa hàng / người bán chỉ thấy và sửa media của nhánh
// mình; độ sâu `own` = media mình tải lên.
use autoxide_kernel::access::{Permission, Risk};

pub(crate) const MEDIA_READ: Permission = Permission::new(
    "media.read",
    "See the media library: every item (private ones too), folders and the trash.",
)
.scoped();
pub(crate) const MEDIA_WRITE: Permission = Permission::new(
    "media.write",
    "Upload, edit, replace, move, publish and trash media items; manage folders. Includes seeing \
     any item it changes (a writer can make a private item public), so give it only to people \
     who may also read the library.",
)
.scoped();
pub(crate) const MEDIA_PURGE: Permission =
    Permission::new("media.purge", "Delete media items in the trash for good.")
        .with_risk(Risk::Destructive)
        .scoped();

pub(crate) fn all() -> Vec<Permission> {
    vec![MEDIA_READ, MEDIA_WRITE, MEDIA_PURGE]
}

//! Permission của module `media` (D-086).

use autoxide_kernel::access::Permission;

pub(crate) const MEDIA_READ: Permission = Permission::new(
    "media.read",
    "See the media library: every item (private ones too), folders and the trash.",
);
pub(crate) const MEDIA_WRITE: Permission = Permission::new(
    "media.write",
    "Upload, edit, replace, move, publish and trash media items; manage folders.",
);
pub(crate) const MEDIA_PURGE: Permission =
    Permission::new("media.purge", "Delete media items in the trash for good.");

pub(crate) fn all() -> Vec<Permission> {
    vec![MEDIA_READ, MEDIA_WRITE, MEDIA_PURGE]
}

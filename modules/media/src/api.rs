//! Contract of the `media` module for other modules (e.g. the upcoming content module): fetch ONE or
//! MANY items by ID at once — a page with 50 images does not need 50 calls.
//!
//! Obtain it with `context.require::<dyn Media>()` in `Module::start` (register AFTER `media`).
//! Items in the trash count as nonexistent. This file does not reference the module's internals.

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantContext;
use uuid::Uuid;

/// Fake media library for other modules' tests (feature `testing`).
#[cfg(feature = "testing")]
pub mod testing;

/// Maximum number of IDs per [`Media::find_many`] call.
pub const MAX_IDS: usize = 500;

/// Anyone can download the content without signing in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaVisibility {
    /// Only those with `media.read`.
    Private,
    /// Anyone with the link can download.
    Public,
}

/// What other modules need to display an item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaRef {
    pub id: Uuid,
    pub visibility: MediaVisibility,
    /// Content type, e.g. `image/webp`.
    pub content_type: String,
    /// Size (pixels), only for raster images.
    pub width: Option<i32>,
    pub height: Option<i32>,
    /// Alternative text (the `alt` of the `<img>` tag); empty if not set.
    pub alt: String,
    /// Default `alt` written from an AI suggestion (Law 134/2025/QH15, D-362): the UI labels it.
    pub alt_ai_assisted: bool,
    /// Download path of the content, with a version (`/api/v1/media/<id>/content?v=<version>`): it changes
    /// when the content changes, so it can be cached for a long time. No sign-in needed when `Public`.
    pub url: String,
    /// Finished thumbnails, narrowest first (for `srcset`); empty if not a JPEG, PNG,
    /// GIF or WebP image, or not finished yet.
    pub variants: Vec<MediaVariant>,
}

/// A thumbnail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaVariant {
    pub width: i32,
    pub height: i32,
    /// The item's `url` plus `&w=<width>`: the JPEG version (PNG for images with transparent regions).
    pub url: String,
    /// The WebP version of the same size (`url` plus `&format=webp`), much lighter; `None` when not made yet
    /// (images uploaded before WebP existed, until `autoxide task media.refresh_variants` runs).
    pub webp_url: Option<String>,
    /// The AVIF version of the same size (`url` plus `&format=avif`, step 39 B8); `None` when not made or the
    /// server does not enable `AUTOXIDE_MEDIA_AVIF`.
    pub avif_url: Option<String>,
}

/// An image prepared to be sent to an AI model ([`Media::ai_image`]).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AiImage {
    /// `image/jpeg` hay `image/png`.
    pub media_type: String,
    pub bytes: Vec<u8>,
}

impl AiImage {
    pub fn new(media_type: impl Into<String>, bytes: Vec<u8>) -> Self {
        AiImage { media_type: media_type.into(), bytes }
    }
}

#[async_trait::async_trait]
pub trait Media: Send + Sync {
    /// Items whose ID is in `ids` and not in the trash, in the order of `ids` (IDs that do not
    /// exist are skipped). Two queries for all IDs (items, thumbnails). More than [`MAX_IDS`] IDs is an
    /// error.
    async fn find_many(&self, tenant: &TenantContext, ids: &[Uuid])
    -> Result<Vec<MediaRef>, Error>;

    /// Like [`find_many`](Self::find_many), with `alt` in language `locale` (BCP 47 code, e.g. the language
    /// of the post being read): the translation if the item has one, otherwise the default `alt` (D-202).
    /// Adds one query for all IDs.
    async fn find_many_in(
        &self,
        tenant: &TenantContext,
        ids: &[Uuid],
        locale: &str,
    ) -> Result<Vec<MediaRef>, Error> {
        let _ = locale;
        self.find_many(tenant, ids).await
    }

    /// Item `id`, or `None` if it does not exist / is in the trash.
    async fn find(&self, tenant: &TenantContext, id: Uuid) -> Result<Option<MediaRef>, Error> {
        Ok(self.find_many(tenant, &[id]).await?.into_iter().next())
    }

    /// Uploader of the items whose ID is in `ids` and not in the trash (`None`: uploaded by the system); IDs
    /// that do not exist are absent. Lets other modules check that an attachment belongs to the sender (e.g.
    /// support tickets, D-202). Old implementations do not know the uploader: every item is `None`.
    async fn uploaders(
        &self,
        tenant: &TenantContext,
        ids: &[Uuid],
    ) -> Result<Vec<(Uuid, Option<Uuid>)>, Error> {
        Ok(self.find_many(tenant, ids).await?.into_iter().map(|item| (item.id, None)).collect())
    }

    /// Image `id` for other modules to send to an AI model (D-109, e.g. `catalog` creating a draft product
    /// from an image — D-229): a thumbnail made by the server, longest side at most 1280 px, with metadata
    /// (EXIF) removed — never the original file. `None` when it does not exist or is in the trash. Not a
    /// readable image: error `media.ai_unsupported_image`; a private image when the server does not allow
    /// sending private images: `media.ai_private_not_allowed`. Reads storage: call OUTSIDE a write
    /// transaction.
    ///
    /// The default implementation returns `None` (the fake in other modules' tests does not need to implement it).
    async fn ai_image(&self, tenant: &TenantContext, id: Uuid) -> Result<Option<AiImage>, Error> {
        let _ = (tenant, id);
        Ok(None)
    }
}

- AVIF copies of images (step 39): with `AUTOXIDE_MEDIA_AVIF=true` (off by default) a background job makes an AVIF
  copy of every smaller copy of an image, about a third lighter than the WebP copy at the same quality. Each variant
  gains `avif_url` and `avif_size` (`null` until made); `…/content?…&w=…&format=avif` serves it, falling back to the
  WebP copy until then. `autoxide task media.refresh_variants` queues copies for older images.

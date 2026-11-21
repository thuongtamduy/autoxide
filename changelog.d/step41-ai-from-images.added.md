- Module `catalog`: draft a product from 1 to 8 photos of the media library with AI
  (`POST /products/from-images`, runs in the background). The result is always a draft product
  marked `ai_assisted`, with texts, a category of your own tree and the attribute values the model
  was sure of; prices, stock and the SKU are left for you.
- Module `media`: other modules can get an image prepared for AI (`Media::ai_image`: a thumbnail
  without metadata, never the original).

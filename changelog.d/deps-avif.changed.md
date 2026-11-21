- Module `media`: the AVIF encoder is now an opt-in build feature (`cargo build --release
  --features avif`), because it depends on an unmaintained build-time library. The default build
  and the Docker image have no AVIF encoder; with `AUTOXIDE_MEDIA_AVIF=true` such a build refuses
  to start and explains how to build with AVIF. AVIF copies already stored are still served.

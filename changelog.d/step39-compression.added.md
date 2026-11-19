- Response compression (step 39): JSON and text responses of 1 KiB or more are compressed with brotli or gzip
  when the client accepts it (a page of 20 posts goes from 136 KB to about 26 KB). Images and other files are never
  compressed again. `AUTOXIDE_HTTP_COMPRESSION=false` turns it off when a reverse proxy or CDN already compresses.

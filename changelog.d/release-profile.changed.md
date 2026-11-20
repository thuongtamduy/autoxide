- The release binary is about half the size (377 MB → 198 MB; the Docker image shrinks by the same
  amount) and slightly faster: release builds strip symbols and use thin link-time optimization
  with one codegen unit. Panic messages still name the file and line; backtraces no longer name
  functions.

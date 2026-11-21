- Module `files`: S3 storage now uses the small `rusty-s3` signing library over the existing HTTP
  client, and local disk storage is built in, instead of `object_store` (which pulled in an
  unmaintained crate). Settings, stored files and behaviour are unchanged, including the server's
  IAM role on AWS when no access key is set.

//! Cấu hình của module `files`. Biến riêng mang tiền tố `AUTOXIDE_FILES_`; thông tin
//! đăng nhập S3 dùng tên chuẩn của AWS (`AWS_…`), như mọi công cụ S3 khác.

use std::path::PathBuf;

use autoxide_kernel::config::{ConfigReader, Invalid, Setting};

pub(crate) const DIR: Setting = Setting::new(
    "AUTOXIDE_FILES_DIR",
    "Directory where uploaded files are stored when AUTOXIDE_FILES_S3_BUCKET is unset; created \
     when missing. Back it up together with the database.",
)
.with_default("data/files");

pub(crate) const S3_BUCKET: Setting = Setting::new(
    "AUTOXIDE_FILES_S3_BUCKET",
    "Store files in this S3 bucket (or a bucket of an S3-compatible service, with \
     AWS_ENDPOINT_URL) instead of AUTOXIDE_FILES_DIR. The bucket must exist.",
)
.with_example("my-app-files");

pub(crate) const S3_PREFIX: Setting = Setting::new(
    "AUTOXIDE_FILES_S3_PREFIX",
    "Folder inside the bucket under which files are stored, so one bucket can serve several \
     deployments. Empty: the root of the bucket.",
)
.with_example("production");

pub(crate) const AWS_REGION: Setting = Setting::new(
    "AWS_REGION",
    "Region of the S3 bucket. S3-compatible services usually accept any value.",
)
.with_default("us-east-1");

pub(crate) const AWS_ENDPOINT_URL: Setting = Setting::new(
    "AWS_ENDPOINT_URL",
    "Address of an S3-compatible service (Cloudflare R2, Backblaze B2, Garage, SeaweedFS…). \
     Unset: Amazon S3.",
)
.with_example("https://<account>.r2.cloudflarestorage.com");

pub(crate) const AWS_ACCESS_KEY_ID: Setting = Setting::new(
    "AWS_ACCESS_KEY_ID",
    "Access key for the S3 bucket. Unset on AWS: the server's IAM role is used.",
)
.with_example("AKIAIOSFODNN7EXAMPLE");

pub(crate) const AWS_SECRET_ACCESS_KEY: Setting =
    Setting::new("AWS_SECRET_ACCESS_KEY", "Secret of AWS_ACCESS_KEY_ID.")
        .secret()
        .with_example("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY");

/// Nơi lưu file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Backend {
    /// Thư mục trên ổ đĩa của máy chủ.
    Local {
        dir: PathBuf,
    },
    S3(S3Config),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct S3Config {
    pub bucket: String,
    pub prefix: Option<String>,
    pub region: String,
    pub endpoint: Option<String>,
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct FilesConfig {
    pub backend: Backend,
}

impl FilesConfig {
    pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<Self, Invalid> {
        let dir = reader.get_with(&DIR, |raw| Ok(PathBuf::from(raw)));
        let bucket = reader.optional_with(&S3_BUCKET, |raw| Ok(raw.trim().to_owned()));
        let prefix = reader.optional_with(&S3_PREFIX, |raw| {
            let prefix = raw.trim().trim_matches('/');
            if prefix.chars().all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c)) {
                Ok(prefix.to_owned())
            } else {
                Err("may contain only letters, digits, `-`, `_`, `.` and `/`".to_owned())
            }
        });
        let region = reader.get_with(&AWS_REGION, |raw| Ok(raw.trim().to_owned()));
        let endpoint = reader.optional_with(&AWS_ENDPOINT_URL, |raw| {
            let url = raw.trim().trim_end_matches('/');
            if url.starts_with("https://") || url.starts_with("http://") {
                Ok(url.to_owned())
            } else {
                Err("must be an `https://` (or `http://`) URL".to_owned())
            }
        });
        let access_key_id = reader.optional_with(&AWS_ACCESS_KEY_ID, |raw| Ok(raw.to_owned()));
        let secret_access_key =
            reader.optional_with(&AWS_SECRET_ACCESS_KEY, |raw| Ok(raw.to_owned()));
        let (dir, bucket, prefix, region, endpoint) = (dir?, bucket?, prefix?, region?, endpoint?);
        let (access_key_id, secret_access_key) = (access_key_id?, secret_access_key?);
        let backend = match bucket {
            None => Backend::Local { dir },
            Some(bucket) => Backend::S3(S3Config {
                bucket,
                prefix: prefix.filter(|prefix| !prefix.is_empty()),
                region,
                endpoint,
                access_key_id,
                secret_access_key,
            }),
        };
        Ok(FilesConfig { backend })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_kernel::config::ConfigSource;

    fn read(pairs: &[(&str, &str)]) -> Result<FilesConfig, String> {
        let source = ConfigSource::from_pairs(pairs.iter().copied());
        let mut reader = ConfigReader::new(&source);
        let config = FilesConfig::read(&mut reader);
        reader.finish(config).map(|(config, _)| config).map_err(|errors| errors.to_string())
    }

    #[test]
    fn mac_dinh_luu_tren_o_dia() {
        let config = read(&[]).unwrap();

        assert_eq!(config.backend, Backend::Local { dir: PathBuf::from("data/files") });
    }

    #[test]
    fn co_bucket_thi_dung_s3() {
        let config = read(&[
            ("AUTOXIDE_FILES_S3_BUCKET", "files"),
            ("AUTOXIDE_FILES_S3_PREFIX", "/prod/"),
            ("AWS_ENDPOINT_URL", "http://127.0.0.1:9000/"),
            ("AWS_ACCESS_KEY_ID", "key"),
            ("AWS_SECRET_ACCESS_KEY", "secret"),
        ])
        .unwrap();

        let Backend::S3(s3) = config.backend else { panic!("expected S3") };
        assert_eq!(s3.bucket, "files");
        assert_eq!(s3.prefix.as_deref(), Some("prod"));
        assert_eq!(s3.region, "us-east-1");
        assert_eq!(s3.endpoint.as_deref(), Some("http://127.0.0.1:9000"));
    }

    #[test]
    fn gia_tri_sai_bi_tu_choi() {
        let error =
            read(&[("AWS_ENDPOINT_URL", "minio:9000"), ("AUTOXIDE_FILES_S3_PREFIX", "a b")])
                .unwrap_err();

        assert!(error.contains("AWS_ENDPOINT_URL"), "{error}");
        assert!(error.contains("AUTOXIDE_FILES_S3_PREFIX"), "{error}");
    }
}

//! Cấu hình của module `files`. Biến riêng mang tiền tố `AUTOXIDE_FILES_`; thông tin
//! đăng nhập S3 dùng tên chuẩn của AWS (`AWS_…`), như mọi công cụ S3 khác.

use std::path::PathBuf;

use autoxide_kernel::config::{ConfigReader, Invalid, Setting, parse_in_range};

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

pub(crate) const MAX_BYTES: Setting = Setting::new(
    "AUTOXIDE_FILES_MAX_BYTES",
    "Largest file that can be uploaded, in bytes (1 to 5368709120). Larger uploads are \
     answered with 413. Big files also need enough time: raise \
     AUTOXIDE_HTTP_REQUEST_TIMEOUT_SECONDS for slow clients.",
)
.with_default("26214400");

pub(crate) const ALLOWED_TYPES: Setting = Setting::new(
    "AUTOXIDE_FILES_ALLOWED_TYPES",
    "Media types that may be uploaded, comma-separated; `image/*` allows every image type. \
     Unset: every type. Other types are answered with 415.",
)
.with_example("image/*,application/pdf,text/plain");

pub(crate) const DIRECT_MAX_BYTES: Setting = Setting::new(
    "AUTOXIDE_FILES_DIRECT_MAX_BYTES",
    "Largest file that can be uploaded straight to S3 with `POST /api/v1/files/uploads`, in bytes \
     (1 to 5368709120, the most one S3 PUT takes). Only used when files are stored on S3.",
)
.with_default("5368709120");

pub(crate) const DIRECT_UPLOAD_SECONDS: Setting = Setting::new(
    "AUTOXIDE_FILES_DIRECT_UPLOAD_SECONDS",
    "How long a link to upload straight to S3 stays valid, in seconds (60 to 3600). Uploads not \
     completed within an hour after that are removed.",
)
.with_default("900");

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
    pub limits: Limits,
    pub direct: Direct,
}

/// Tải thẳng lên S3 (bước 40 P4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Direct {
    pub max_bytes: u64,
    pub ttl: std::time::Duration,
}

impl Default for Direct {
    fn default() -> Self {
        Direct { max_bytes: 5 * 1024 * 1024 * 1024, ttl: std::time::Duration::from_secs(900) }
    }
}

/// Giới hạn của file tải lên.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Limits {
    pub max_bytes: u64,
    /// `None`: mọi loại. Mỗi mục là `loại/kiểu` hoặc `loại/*`, chữ thường.
    pub allowed_types: Option<Vec<String>>,
}

impl Limits {
    /// `content_type` (đã chuẩn hoá) có được phép không.
    pub(crate) fn allows(&self, content_type: &str) -> bool {
        let Some(allowed) = &self.allowed_types else {
            return true;
        };
        allowed.iter().any(|pattern| match pattern.strip_suffix("/*") {
            Some(kind) => content_type.split('/').next() == Some(kind),
            None => pattern == content_type,
        })
    }
}

/// Loại nội dung dạng `loại/kiểu` (RFC 6838: chữ, số và `!#$&-^_.+`), chữ thường, bỏ
/// tham số (`; charset=…`). `None` nếu sai dạng.
pub(crate) fn normalize_type(raw: &str) -> Option<String> {
    let essence = raw.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
    let (kind, subtype) = essence.split_once('/')?;
    let token = |part: &str| {
        (1..=127).contains(&part.len())
            && part.chars().all(|c| c.is_ascii_alphanumeric() || "!#$&-^_.+".contains(c))
    };
    (token(kind) && token(subtype)).then_some(essence)
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
        let max_bytes =
            reader.get_with(&MAX_BYTES, |raw| parse_in_range(raw, 1u64, 5 * 1024 * 1024 * 1024));
        let allowed_types = reader.optional_with(&ALLOWED_TYPES, |raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(|item| {
                    let wildcard = item.strip_suffix("/*").map(|kind| format!("{kind}/x"));
                    let checked = normalize_type(wildcard.as_deref().unwrap_or(item));
                    match (checked, wildcard.is_some()) {
                        (Some(_), true) => Ok(item.to_ascii_lowercase()),
                        (Some(normalized), false) => Ok(normalized),
                        (None, _) => Err(format!(
                            "`{item}` is not a media type like `application/pdf` or `image/*`"
                        )),
                    }
                })
                .collect::<Result<Vec<_>, _>>()
        });
        let direct_max = reader.get_with(&DIRECT_MAX_BYTES, |raw| {
            parse_in_range(raw, 1u64, autoxide_kernel::upload::MAX_SINGLE_PUT_BYTES)
        });
        let direct_seconds =
            reader.get_with(&DIRECT_UPLOAD_SECONDS, |raw| parse_in_range(raw, 60u64, 3600));
        let (dir, bucket, prefix, region, endpoint) = (dir?, bucket?, prefix?, region?, endpoint?);
        let direct =
            Direct { max_bytes: direct_max?, ttl: std::time::Duration::from_secs(direct_seconds?) };
        let limits = Limits { max_bytes: max_bytes?, allowed_types: allowed_types? };
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
        Ok(FilesConfig { backend, limits, direct })
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
        assert_eq!(config.limits, Limits { max_bytes: 26_214_400, allowed_types: None });
        assert!(config.limits.allows("application/x-anything"));
    }

    #[test]
    fn gioi_han_loai_file() {
        let config =
            read(&[("AUTOXIDE_FILES_ALLOWED_TYPES", "Image/*, application/PDF ,")]).unwrap();

        let limits = config.limits;
        assert_eq!(limits.allowed_types, Some(vec!["image/*".into(), "application/pdf".into()]));
        assert!(limits.allows("image/png"));
        assert!(limits.allows("application/pdf"));
        assert!(!limits.allows("application/zip"));
        assert!(!limits.allows("imagex/png"));
        assert!(read(&[("AUTOXIDE_FILES_ALLOWED_TYPES", "pdf")]).is_err());
        assert!(read(&[("AUTOXIDE_FILES_MAX_BYTES", "0")]).is_err());
    }

    #[test]
    fn chuan_hoa_loai_noi_dung() {
        assert_eq!(normalize_type("Text/Plain; charset=UTF-8").as_deref(), Some("text/plain"));
        assert_eq!(
            normalize_type("application/vnd.api+json").as_deref(),
            Some("application/vnd.api+json")
        );
        assert_eq!(normalize_type("text"), None);
        assert_eq!(normalize_type("text/"), None);
        assert_eq!(normalize_type("te xt/plain"), None);
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

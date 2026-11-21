//! Nơi lưu S3 (hay dịch vụ tương thích S3: R2, MinIO…) qua `rusty-s3` + `reqwest` (D-20261121-s3-client):
//! `rusty-s3` dựng và ký request (AWS SigV4, link có hạn), `reqwest` gửi. Địa chỉ kiểu đường dẫn
//! (`<endpoint>/<bucket>/<khoá>`) như `object_store` trước đây, nên dịch vụ tương thích nào cũng dùng được.
//!
//! Credential: khoá khai ở cấu hình, không có thì lấy credential tạm của IAM role qua dịch vụ metadata của
//! EC2 (IMDSv2), tự làm mới trước khi hết hạn — như `object_store` dựng bằng `AmazonS3Builder::new()`.

use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::error::Error;
use autoxide_kernel::storage::{
    ByteStream, ObjectInfo, PresignedUpload, StoredFile, UploadConstraints,
};
use bytes::{Bytes, BytesMut};
use futures_util::{StreamExt as _, TryStreamExt as _};
use reqwest::{Method, StatusCode, header};
use rusty_s3::actions::{
    CompleteMultipartUpload, CreateMultipartUpload, ListObjectsV2, S3Action as _,
};
use rusty_s3::{Bucket, Credentials, UrlStyle};
use url::Url;

use crate::config::S3Config;

/// Ghi theo từng phần (multipart) chừng này byte; file nhỏ hơn ghi một lần. S3 đòi mỗi phần (trừ phần cuối)
/// ít nhất 5 MiB.
const PART_SIZE: usize = 8 * 1024 * 1024;

/// Hạn của link ký cho request của chính máy chủ: chỉ cần còn hạn lúc request bắt đầu.
const SIGN_FOR: Duration = Duration::from_secs(15 * 60);

/// Số lần thử một request gặp lỗi tạm (mạng, 5xx, 429), cách nhau 100 ms, 200 ms…
const ATTEMPTS: u32 = 3;

/// Dịch vụ metadata của EC2 (IMDS).
const METADATA_ENDPOINT: &str = "http://169.254.169.254";

/// Làm mới credential tạm trước khi hết hạn chừng này.
const REFRESH_BEFORE: Duration = Duration::from_secs(5 * 60);

pub(super) struct S3Store {
    inner: Arc<Inner>,
}

struct Inner {
    bucket: Bucket,
    credentials: CredentialSource,
    http: reqwest::Client,
}

fn storage_error(message: impl std::fmt::Display) -> Error {
    Error::internal(format!("file storage: {message}"))
}

impl S3Store {
    pub(super) fn open(config: &S3Config) -> Result<Self, Error> {
        // HTTP client của S3 dùng rustls với provider chung của chương trình.
        autoxide_kernel::tls::install_crypto_provider();
        let endpoint = match &config.endpoint {
            Some(endpoint) => endpoint.clone(),
            None => format!("https://s3.{}.amazonaws.com", config.region),
        };
        let endpoint = Url::parse(&endpoint).map_err(|error| {
            Error::internal(format!("cannot set up S3: endpoint `{endpoint}`: {error}"))
        })?;
        let bucket =
            Bucket::new(endpoint, UrlStyle::Path, config.bucket.clone(), config.region.clone())
                .map_err(|error| Error::internal(format!("cannot set up S3: {error}")))?;
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            // Không đặt hạn tổng: tải file lớn lâu là bình thường. Hạn mỗi lần đọc bắt mạng treo.
            .read_timeout(Duration::from_secs(60))
            .build()
            .map_err(|error| Error::internal(format!("cannot set up S3: {error}")))?;
        let credentials = match (&config.access_key_id, &config.secret_access_key) {
            (Some(key), Some(secret)) => {
                CredentialSource::Static(Arc::new(Credentials::new(key.clone(), secret.clone())))
            }
            (None, None) => CredentialSource::Instance(InstanceCredentials::new(METADATA_ENDPOINT)),
            _ => {
                return Err(Error::internal(
                    "cannot set up S3: set both AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY, or neither",
                ));
            }
        };
        Ok(S3Store { inner: Arc::new(Inner { bucket, credentials, http }) })
    }

    pub(super) async fn put(&self, object: &str, mut content: ByteStream) -> Result<u64, Error> {
        let mut buffer = BytesMut::new();
        let mut written: u64 = 0;
        // Tạo multipart upload khi đã có hơn một phần. Bọc trong `Multipart`: bị bỏ giữa chừng (lỗi, request
        // bị huỷ) thì phần đã gửi được huỷ — S3 không tự huỷ, các phần nằm lại vô hình mà vẫn tính tiền (D-158).
        let mut multipart: Option<Multipart> = None;
        while let Some(chunk) = content.next().await {
            let chunk = chunk?;
            written += chunk.len() as u64;
            buffer.extend_from_slice(&chunk);
            while buffer.len() >= PART_SIZE {
                let part = buffer.split_to(PART_SIZE).freeze();
                let upload = match &mut multipart {
                    Some(upload) => upload,
                    None => multipart.insert(self.inner.create_multipart(object).await?),
                };
                upload.upload_part(part).await?;
            }
        }
        match multipart {
            None => self.inner.put_object(object, buffer.freeze()).await?,
            Some(mut upload) => {
                if !buffer.is_empty() {
                    upload.upload_part(buffer.freeze()).await?;
                }
                upload.complete().await?;
            }
        }
        Ok(written)
    }

    pub(super) async fn get(&self, object: &str) -> Result<Option<StoredFile>, Error> {
        self.inner.get(object, None).await
    }

    pub(super) async fn get_range(
        &self,
        object: &str,
        range: std::ops::Range<u64>,
    ) -> Result<Option<StoredFile>, Error> {
        if range.start >= range.end {
            return Ok(Some(StoredFile::new(0, Box::pin(futures_util::stream::empty()))));
        }
        self.inner.get(object, Some(range)).await
    }

    pub(super) async fn head(&self, object: &str) -> Result<Option<ObjectInfo>, Error> {
        let credentials = self.inner.credentials.get(&self.inner.http).await?;
        let url = self.inner.bucket.head_object(Some(&credentials), object).sign(SIGN_FOR);
        let response = self.inner.send(Method::HEAD, url, &[], None).await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let response = check(response, "HEAD").await?;
        Ok(Some(ObjectInfo::new(content_length(&response)?)))
    }

    pub(super) async fn delete(&self, object: &str) -> Result<(), Error> {
        let credentials = self.inner.credentials.get(&self.inner.http).await?;
        let url = self.inner.bucket.delete_object(Some(&credentials), object).sign(SIGN_FOR);
        let response = self.inner.send(Method::DELETE, url, &[], None).await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(());
        }
        check(response, "DELETE").await.map(drop)
    }

    /// Mọi khoá đối tượng bắt đầu bằng `prefix`, theo thứ tự tên (S3 trả theo thứ tự byte của khoá).
    pub(super) async fn list(&self, prefix: &str) -> Result<Vec<String>, Error> {
        let mut keys = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let credentials = self.inner.credentials.get(&self.inner.http).await?;
            let mut action = ListObjectsV2::new(&self.inner.bucket, Some(&credentials));
            if !prefix.is_empty() {
                action.with_prefix(prefix);
            }
            if let Some(token) = &token {
                action.with_continuation_token(token.as_str());
            }
            let url = action.sign(SIGN_FOR);
            let response = self.inner.send(Method::GET, url, &[], None).await?;
            let body = check(response, "LIST").await?.text().await.map_err(storage_error)?;
            let page = ListObjectsV2::parse_response(&body).map_err(storage_error)?;
            keys.extend(page.contents.into_iter().map(|object| object.key));
            match page.next_continuation_token {
                Some(next) => token = Some(next),
                None => break,
            }
        }
        keys.sort();
        Ok(keys)
    }

    /// Link `PUT` ký SigV4 (bước 40 P4), ký kèm `content-type` và `x-amz-checksum-sha256`: S3 từ chối request
    /// thiếu hay sai một trong hai, và từ chối nội dung khác hash — tức khác cả cỡ. `content-length` không ký
    /// (như trước); nó vẫn được trả để client gửi. Dịch vụ tương thích S3 có thể không kiểm hash: lệnh xác
    /// nhận luôn đọc lại, đo cỡ và băm (D-379).
    pub(super) async fn presign_upload(
        &self,
        object: &str,
        constraints: &UploadConstraints,
        ttl: Duration,
    ) -> Result<PresignedUpload, Error> {
        use base64::Engine as _;
        let credentials = self.inner.credentials.get(&self.inner.http).await?;
        let checksum = base64::engine::general_purpose::STANDARD.encode(constraints.sha256);
        let issued = chrono::Utc::now();
        let mut action = self.inner.bucket.put_object(Some(&credentials), object);
        action.headers_mut().insert("content-type", constraints.content_type.clone());
        action.headers_mut().insert("x-amz-checksum-sha256", checksum.clone());
        let url = action.sign(ttl);
        let headers = vec![
            ("content-length".to_owned(), constraints.size.to_string()),
            ("content-type".to_owned(), constraints.content_type.clone()),
            ("x-amz-checksum-sha256".to_owned(), checksum),
        ];
        let expires_at = issued + chrono::Duration::from_std(ttl).map_err(Error::internal)?;
        Ok(PresignedUpload::new("PUT", url.to_string(), headers, expires_at))
    }
}

impl Inner {
    /// Gửi một request, thử lại lỗi tạm (mạng, 5xx, 429). Mọi request ở đây gửi lại được (cùng thân).
    async fn send(
        &self,
        method: Method,
        url: Url,
        headers: &[(&str, String)],
        body: Option<Bytes>,
    ) -> Result<reqwest::Response, Error> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let mut request = self.http.request(method.clone(), url.clone());
            for (name, value) in headers {
                request = request.header(*name, value.as_str());
            }
            if let Some(body) = &body {
                request = request.body(body.clone());
            }
            let transient = match request.send().await {
                Ok(response)
                    if response.status().is_server_error()
                        || response.status() == StatusCode::TOO_MANY_REQUESTS =>
                {
                    if attempt >= ATTEMPTS {
                        return Ok(response);
                    }
                    format!("HTTP {}", response.status())
                }
                Ok(response) => return Ok(response),
                Err(error) if attempt >= ATTEMPTS => return Err(storage_error(error)),
                Err(error) => error.to_string(),
            };
            tracing::warn!(attempt, error = %transient, "S3 request failed, retrying");
            tokio::time::sleep(Duration::from_millis(100 << attempt)).await;
        }
    }

    async fn get(
        &self,
        object: &str,
        range: Option<std::ops::Range<u64>>,
    ) -> Result<Option<StoredFile>, Error> {
        let credentials = self.credentials.get(&self.http).await?;
        let url = self.bucket.get_object(Some(&credentials), object).sign(SIGN_FOR);
        let headers: Vec<(&str, String)> = match &range {
            // `Range` không ký: S3 nhận header không ký đi kèm link ký sẵn.
            Some(range) => vec![("range", format!("bytes={}-{}", range.start, range.end - 1))],
            None => Vec::new(),
        };
        let response = self.send(Method::GET, url, &headers, None).await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let response = check(response, "GET").await?;
        let size = content_length(&response)?;
        let content = response.bytes_stream().map_err(storage_error).boxed();
        Ok(Some(StoredFile::new(size, content)))
    }

    async fn put_object(&self, object: &str, body: Bytes) -> Result<(), Error> {
        let credentials = self.credentials.get(&self.http).await?;
        let url = self.bucket.put_object(Some(&credentials), object).sign(SIGN_FOR);
        let response = self.send(Method::PUT, url, &[], Some(body)).await?;
        check(response, "PUT").await.map(drop)
    }

    async fn create_multipart(self: &Arc<Self>, object: &str) -> Result<Multipart, Error> {
        let credentials = self.credentials.get(&self.http).await?;
        let url =
            CreateMultipartUpload::new(&self.bucket, Some(&credentials), object).sign(SIGN_FOR);
        let response = self.send(Method::POST, url, &[], None).await?;
        let body =
            check(response, "CreateMultipartUpload").await?.text().await.map_err(storage_error)?;
        let created = CreateMultipartUpload::parse_response(&body).map_err(storage_error)?;
        Ok(Multipart {
            inner: Arc::clone(self),
            object: object.to_owned(),
            upload_id: created.upload_id().to_owned(),
            etags: Vec::new(),
            done: false,
        })
    }
}

/// Một multipart upload đang dở. Bị bỏ trước [`Multipart::complete`] thì huỷ (xem `put`).
struct Multipart {
    inner: Arc<Inner>,
    object: String,
    upload_id: String,
    etags: Vec<String>,
    done: bool,
}

impl Multipart {
    async fn upload_part(&mut self, part: Bytes) -> Result<(), Error> {
        let number = u16::try_from(self.etags.len() + 1)
            .map_err(|_| storage_error("file too large: more than 65535 parts"))?;
        let credentials = self.inner.credentials.get(&self.inner.http).await?;
        let url = self
            .inner
            .bucket
            .upload_part(Some(&credentials), &self.object, number, &self.upload_id)
            .sign(SIGN_FOR);
        let response = self.inner.send(Method::PUT, url, &[], Some(part)).await?;
        let response = check(response, "UploadPart").await?;
        let etag = response
            .headers()
            .get(header::ETAG)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| storage_error("UploadPart: no ETag in the answer"))?;
        self.etags.push(etag.to_owned());
        Ok(())
    }

    async fn complete(mut self) -> Result<(), Error> {
        let credentials = self.inner.credentials.get(&self.inner.http).await?;
        let action = CompleteMultipartUpload::new(
            &self.inner.bucket,
            Some(&credentials),
            &self.object,
            &self.upload_id,
            self.etags.iter().map(String::as_str),
        );
        let url = action.sign(SIGN_FOR);
        let body = Bytes::from(action.body());
        let response = self.inner.send(Method::POST, url, &[], Some(body)).await?;
        let text = check(response, "CompleteMultipartUpload")
            .await?
            .text()
            .await
            .map_err(storage_error)?;
        // S3 có thể trả 200 mà thân là lỗi (lỗi xảy ra sau khi đã gửi header).
        if text.contains("<Error>") {
            return Err(storage_error(format!("CompleteMultipartUpload: {}", s3_error(&text))));
        }
        self.done = true;
        Ok(())
    }
}

impl Drop for Multipart {
    fn drop(&mut self) {
        // `Drop` không chờ được: giao việc huỷ cho runtime (nếu còn).
        if self.done {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else { return };
        let inner = Arc::clone(&self.inner);
        let object = std::mem::take(&mut self.object);
        let upload_id = std::mem::take(&mut self.upload_id);
        runtime.spawn(async move {
            let abort = async {
                let credentials = inner.credentials.get(&inner.http).await?;
                let url = inner
                    .bucket
                    .abort_multipart_upload(Some(&credentials), &object, &upload_id)
                    .sign(SIGN_FOR);
                let response = inner.send(Method::DELETE, url, &[], None).await?;
                check(response, "AbortMultipartUpload").await.map(drop)
            };
            if let Err(error) = abort.await {
                tracing::warn!(error = %error, "cannot abort a partial upload");
            }
        });
    }
}

/// Response thành công thì trả lại; không thì lỗi kèm mã và câu của S3.
async fn check(response: reqwest::Response, operation: &str) -> Result<reqwest::Response, Error> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    Err(storage_error(format!("S3 {operation} answered {status}: {}", s3_error(&body))))
}

/// `Code: Message` trong thân lỗi XML của S3 (rỗng: thân trống, vd lỗi của `HEAD`).
fn s3_error(body: &str) -> String {
    let field = |name: &str| {
        let open = format!("<{name}>");
        let close = format!("</{name}>");
        body.split_once(&open).and_then(|(_, rest)| rest.split_once(&close)).map(|(value, _)| value)
    };
    match (field("Code"), field("Message")) {
        (Some(code), Some(message)) => format!("{code}: {message}"),
        (Some(code), None) => code.to_owned(),
        _ => body.chars().take(200).collect(),
    }
}

fn content_length(response: &reqwest::Response) -> Result<u64, Error> {
    response
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| storage_error("S3 answer without Content-Length"))
}

/// Nguồn credential: khoá cố định, hay credential tạm của IAM role.
enum CredentialSource {
    Static(Arc<Credentials>),
    Instance(InstanceCredentials),
}

impl CredentialSource {
    async fn get(&self, http: &reqwest::Client) -> Result<Arc<Credentials>, Error> {
        match self {
            CredentialSource::Static(credentials) => Ok(Arc::clone(credentials)),
            CredentialSource::Instance(instance) => instance.get(http).await,
        }
    }
}

/// Credential tạm của IAM role gắn với máy EC2 (IMDSv2: lấy token phiên rồi mới đọc credential), giữ tới
/// gần hạn.
struct InstanceCredentials {
    endpoint: String,
    // `tokio::sync::Mutex`: giữ khoá qua `await` (lúc gọi IMDS) để chỉ một request đi lấy credential mới.
    cached: tokio::sync::Mutex<Option<(Arc<Credentials>, jiff::Timestamp)>>,
}

impl InstanceCredentials {
    fn new(endpoint: &str) -> Self {
        InstanceCredentials {
            endpoint: endpoint.trim_end_matches('/').to_owned(),
            cached: tokio::sync::Mutex::new(None),
        }
    }

    async fn get(&self, http: &reqwest::Client) -> Result<Arc<Credentials>, Error> {
        let mut cached = self.cached.lock().await;
        let fresh_until = jiff::Timestamp::now()
            .checked_add(jiff::SignedDuration::try_from(REFRESH_BEFORE).unwrap_or_default())
            .unwrap_or_else(|_| jiff::Timestamp::now());
        if let Some((credentials, expiration)) = cached.as_ref()
            && *expiration > fresh_until
        {
            return Ok(Arc::clone(credentials));
        }
        let (credentials, expiration) = self.fetch(http).await?;
        let credentials = Arc::new(credentials);
        *cached = Some((Arc::clone(&credentials), expiration));
        Ok(credentials)
    }

    async fn fetch(&self, http: &reqwest::Client) -> Result<(Credentials, jiff::Timestamp), Error> {
        let failed = |error: &dyn std::fmt::Display| {
            storage_error(format!(
                "no S3 credentials: AWS_ACCESS_KEY_ID is not set and the EC2 instance metadata service \
                 gave none ({error})"
            ))
        };
        let token = http
            .put(format!("{}/latest/api/token", self.endpoint))
            .header("x-aws-ec2-metadata-token-ttl-seconds", "21600")
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|error| failed(&error))?
            .text()
            .await
            .map_err(|error| failed(&error))?;
        let base = format!("{}/latest/meta-data/iam/security-credentials/", self.endpoint);
        let read = |url: String| {
            http.get(url)
                .header("x-aws-ec2-metadata-token", token.as_str())
                .timeout(Duration::from_secs(2))
                .send()
        };
        let roles = read(base.clone())
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|error| failed(&error))?
            .text()
            .await
            .map_err(|error| failed(&error))?;
        let role = roles.lines().next().map(str::trim).filter(|role| !role.is_empty());
        let Some(role) = role else {
            return Err(failed(&"no IAM role is attached to this instance"));
        };
        let body = read(format!("{base}{role}"))
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|error| failed(&error))?
            .text()
            .await
            .map_err(|error| failed(&error))?;
        let parsed =
            rusty_s3::credentials::Ec2SecurityCredentialsMetadataResponse::deserialize(&body)
                .map_err(|error| failed(&error))?;
        let expiration = parsed.expiration();
        Ok((parsed.into_credentials(), expiration))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_loi_xml_cua_s3() {
        let body = "<?xml version=\"1.0\"?><Error><Code>NoSuchBucket</Code><Message>The bucket does not \
                    exist</Message></Error>";
        assert_eq!(s3_error(body), "NoSuchBucket: The bucket does not exist");
        assert_eq!(s3_error(""), "");
    }

    /// Dịch vụ metadata giả (IMDSv2) trên cổng cục bộ: đòi token phiên, trả một role và credential của nó.
    #[tokio::test]
    async fn credential_tam_tu_imds_va_giu_toi_gan_han() {
        use axum::routing::{get, put};
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let app = axum::Router::new()
            .route("/latest/api/token", put(|| async { "session-token" }))
            .route(
                "/latest/meta-data/iam/security-credentials/",
                get(|headers: axum::http::HeaderMap| async move {
                    if headers.get("x-aws-ec2-metadata-token").is_some() { "app-role\n" } else { "" }
                }),
            )
            .route(
                "/latest/meta-data/iam/security-credentials/app-role",
                get(move || {
                    counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    async {
                        let expiration = jiff::Timestamp::now() + jiff::SignedDuration::from_hours(6);
                        format!(
                            r#"{{"Code":"Success","AccessKeyId":"AKIA","SecretAccessKey":"secret","Token":"tok","Expiration":"{expiration}"}}"#
                        )
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await });
        autoxide_kernel::tls::install_crypto_provider();
        let http = reqwest::Client::new();
        let instance = InstanceCredentials::new(&format!("http://{address}"));
        let first = instance.get(&http).await.unwrap();
        assert_eq!((first.key(), first.secret(), first.token()), ("AKIA", "secret", Some("tok")));
        // Còn xa hạn: không hỏi lại.
        let again = instance.get(&http).await.unwrap();
        assert!(Arc::ptr_eq(&first, &again));
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);

        let nowhere = InstanceCredentials::new("http://127.0.0.1:9");
        let error = nowhere.get(&http).await.unwrap_err();
        assert!(format!("{error:?}").contains("AWS_ACCESS_KEY_ID is not set"), "{error:?}");
    }
}

//! Test nâng cấp từ bản phát hành trước (đặc tả 31 B1, D-246).
//!
//! Bản cũ chạy bằng image Docker của nó (`TEST_UPGRADE_IMAGE`), ghi dữ liệu mẫu qua API thật, dừng
//! êm; rồi bản hiện tại (binary vừa build) chạy trên **cùng database và cùng thư mục dữ liệu**:
//! `migrate up`, `doctor`, `serve`, đọc lại mọi bản ghi, dùng lại token cũ, ghi thêm vài thứ, chạy
//! `task media.refresh_variants`.
//!
//! Chưa đặt `TEST_UPGRADE_IMAGE` thì test in "skipped" và qua (giống test S3 với
//! `TEST_S3_ENDPOINT`): nó cần Docker và một image đã build, nên không chạy trong `./gate.sh`.
//! `cargo xtask upgrade-test` chuẩn bị image rồi chạy test này trên SQLite và Postgres.
//!
//! Container chạy với `--network host` (chỉ Linux): bản cũ gọi được Postgres của test và test gọi
//! được bản cũ ở `127.0.0.1` mà không cần ánh xạ port.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use autoxide_testkit::TestDatabase;
use sea_orm::DbBackend;
use serde_json::{Value, json};

const IMAGE_VARIABLE: &str = "TEST_UPGRADE_IMAGE";
const EMAIL: &str = "admin@example.com";
const PASSWORD: &str = "correct horse battery staple";
/// Ảnh PNG thật (48 × 32) dùng chung với hướng dẫn.
const PHOTO: &[u8] = include_bytes!("../../../docs/guides/files/photo.png");

/// Thư mục làm việc của một lần chạy: `data/` của nó là `/var/lib/autoxide/data` trong container,
/// và binary hiện tại chạy với thư mục này làm thư mục hiện tại — hai bản thấy cùng `data/`.
struct WorkDir(PathBuf);

impl WorkDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("autoxide-upgrade-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(path.join("data")).unwrap();
        WorkDir(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Container của bản cũ đang chạy; bị gỡ khi ra khỏi phạm vi (cả lúc test hỏng giữa chừng).
struct Container(String);

impl Drop for Container {
    fn drop(&mut self) {
        let _ = Command::new("docker").args(["rm", "-f", &self.0]).output();
    }
}

/// Tiến trình `serve` của bản hiện tại; bị dừng khi ra khỏi phạm vi.
struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Cấu hình chung của hai bản: cùng database, cùng khoá ký access token (token cũ phải còn
/// dùng được), cùng chiều rộng ảnh nhỏ (ảnh mẫu 48 px thì có một bản 24 px).
fn environment(database_url: &str, port: u16) -> Vec<(String, String)> {
    [
        ("DATABASE_URL", database_url.to_owned()),
        // Không phải secret thật: chỉ sống trong lần chạy test này.
        ("AUTOXIDE_AUTH_TOKEN_SECRET", "upgrade-test-".repeat(4)),
        ("AUTOXIDE_MEDIA_IMAGE_WIDTHS", "24".to_owned()),
        ("AUTOXIDE_HTTP_ADDRESS", format!("127.0.0.1:{port}")),
        ("AUTOXIDE_LOG_FORMAT", "json".to_owned()),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_owned(), value))
    .collect()
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn text(output: &Output) -> String {
    format!(
        "status {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn id(flag: &str) -> String {
    let output = Command::new("id").arg(flag).output().unwrap();
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// `docker run` image cũ với cấu hình chung. `detach` thì chạy nền với tên `name`.
fn docker(
    image: &str,
    directory: &Path,
    environment: &[(String, String)],
    arguments: &[&str],
    input: Option<&str>,
    name: Option<&str>,
) -> Output {
    use std::io::Write;

    let mut command = Command::new("docker");
    command.arg("run").args(["--network", "host"]);
    // Image chạy bằng uid 10001; dùng uid của máy để file trong thư mục gắn vào ghi được và xoá được.
    command.args(["--user", &format!("{}:{}", id("-u"), id("-g"))]);
    command.args(["-v", &format!("{}:/var/lib/autoxide/data", directory.join("data").display())]);
    match name {
        Some(name) => command.args(["-d", "--name", name]),
        None => command.args(["--rm", "-i"]),
    };
    for (key, value) in environment {
        command.args(["-e", &format!("{key}={value}")]);
    }
    command.arg(image).args(arguments);
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    if let Some(input) = input {
        stdin.write_all(input.as_bytes()).unwrap();
    }
    drop(stdin);
    child.wait_with_output().unwrap()
}

/// Chạy binary hiện tại trong `directory`, chỉ với các biến `environment`.
fn current(directory: &Path, environment: &[(String, String)], arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_autoxide"))
        .args(arguments)
        .current_dir(directory)
        .env_clear()
        .envs(environment.iter().map(|(key, value)| (key.as_str(), value.as_str())))
        .output()
        .unwrap()
}

/// Phiên bản `(major, minor, patch)` từ dòng `autoxide 0.13.0`.
fn parse_version(line: &str) -> (u64, u64, u64) {
    let number = line.split_whitespace().last().unwrap_or_default();
    let parts: Vec<u64> = number.split('.').filter_map(|part| part.parse().ok()).collect();
    match parts.as_slice() {
        [major, minor, patch, ..] => (*major, *minor, *patch),
        _ => panic!("cannot read a version from `{line}`"),
    }
}

/// Một request tới máy chủ đang chạy; thân trả về đọc thành JSON (rỗng: `null`).
struct Api {
    http: reqwest::Client,
    base: String,
}

struct Answer {
    status: u16,
    etag: Option<String>,
    body: Value,
}

enum Body {
    Json(Value),
    Bytes(&'static str, Vec<u8>),
}

impl Api {
    fn new(port: u16) -> Self {
        // reqwest dùng rustls không kèm provider; chương trình chọn `ring` (`kernel::tls`).
        autoxide_kernel::tls::install_crypto_provider();
        Api { http: reqwest::Client::new(), base: format!("http://127.0.0.1:{port}") }
    }

    async fn call(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        headers: &[(&str, &str)],
        body: Option<Body>,
    ) -> Answer {
        let method = reqwest::Method::from_bytes(method.as_bytes()).unwrap();
        let mut request = self.http.request(method, format!("{}{path}", self.base));
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        match body {
            Some(Body::Json(value)) => {
                request = request
                    .header("content-type", "application/json")
                    .body(serde_json::to_vec(&value).unwrap());
            }
            Some(Body::Bytes(media, bytes)) => {
                request = request.header("content-type", media).body(bytes);
            }
            None => {}
        }
        let response = request.send().await.unwrap();
        let status = response.status().as_u16();
        let etag =
            response.headers().get("etag").and_then(|value| value.to_str().ok()).map(str::to_owned);
        let bytes = response.bytes().await.unwrap();
        let body =
            if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() };
        Answer { status, etag, body }
    }

    async fn get(&self, path: &str, token: &str) -> Answer {
        self.call("GET", path, Some(token), &[], None).await
    }

    async fn post(&self, path: &str, token: Option<&str>, body: Value) -> Answer {
        self.call("POST", path, token, &[], Some(Body::Json(body))).await
    }

    /// Chờ `/health/ready` trả 200 (máy chủ đã nghe và database đã sẵn sàng).
    async fn wait_ready(&self, describe: impl Fn() -> String) {
        let started = Instant::now();
        loop {
            if let Ok(response) = self.http.get(format!("{}/health/ready", self.base)).send().await
                && response.status().as_u16() == 200
            {
                return;
            }
            assert!(
                started.elapsed() < Duration::from_secs(60),
                "server not ready:\n{}",
                describe()
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    async fn login(&self) -> (String, String) {
        let login = self
            .post("/api/v1/auth/login", None, json!({"email": EMAIL, "password": PASSWORD}))
            .await;
        assert_eq!(login.status, 200, "{}", login.body);
        let data = &login.body["data"];
        (
            data["access_token"].as_str().unwrap().to_owned(),
            data["refresh_token"].as_str().unwrap().to_owned(),
        )
    }

    /// Tải ảnh mẫu lên, cho công khai, rồi chờ job làm xong ảnh nhỏ. Trả ID.
    async fn upload_photo(&self, token: &str, name: &str) -> String {
        let created = self
            .call(
                "POST",
                &format!("/api/v1/media?name={name}"),
                Some(token),
                &[],
                Some(Body::Bytes("image/png", PHOTO.to_vec())),
            )
            .await;
        assert_eq!(created.status, 201, "{}", created.body);
        let id = created.body["data"]["id"].as_str().unwrap().to_owned();
        // Bài đã đăng chỉ được dùng ảnh công khai.
        let public = self
            .call(
                "PATCH",
                &format!("/api/v1/media/{id}"),
                Some(token),
                &[("if-match", "*")],
                Some(Body::Json(json!({"visibility": "public"}))),
            )
            .await;
        assert_eq!(public.status, 200, "make public: {}", public.body);
        let started = Instant::now();
        loop {
            let item = self.get(&format!("/api/v1/media/{id}"), token).await;
            if item.body["data"]["variants"].as_array().is_some_and(|variants| !variants.is_empty())
            {
                return id;
            }
            assert!(started.elapsed() < Duration::from_secs(30), "no variants: {}", item.body);
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
}

/// `old` là tập con của `new`: mọi trường bản cũ trả về, bản mới trả về y hệt (bản mới được thêm
/// trường). Mảng phải cùng độ dài, so từng phần tử.
fn contained(old: &Value, new: &Value, path: &str, problems: &mut Vec<String>) {
    match (old, new) {
        (Value::Object(old), Value::Object(new)) => {
            for (key, value) in old {
                match new.get(key) {
                    Some(other) => contained(value, other, &format!("{path}.{key}"), problems),
                    None => problems.push(format!("{path}.{key}: missing after the upgrade")),
                }
            }
        }
        (Value::Array(old), Value::Array(new)) if old.len() == new.len() => {
            for (index, (value, other)) in old.iter().zip(new).enumerate() {
                contained(value, other, &format!("{path}[{index}]"), problems);
            }
        }
        _ if old == new => {}
        _ => problems.push(format!("{path}: {old} before, {new} after")),
    }
}

/// Ghi dữ liệu mẫu qua API của bản cũ. Trả các đường dẫn để đọc lại và ID của bài đã đăng.
async fn write_sample(api: &Api, token: &str, version: (u64, u64, u64)) -> (Vec<String>, String) {
    let mut paths = Vec::new();
    let created = |answer: Answer, what: &str| -> String {
        assert_eq!(answer.status, 201, "{what}: {}", answer.body);
        answer.body["data"]["id"].as_str().unwrap().to_owned()
    };

    let role = api
        .post(
            "/api/v1/auth/roles",
            Some(token),
            json!({"name": "editor", "permissions": ["content.read"]}),
        )
        .await;
    paths.push(format!("/api/v1/auth/roles/{}", created(role, "role")));

    let photo = api.upload_photo(token, "photo.png").await;
    paths.push(format!("/api/v1/media/{photo}"));

    let draft = api
        .post("/api/v1/content/entries", Some(token), json!({"kind": "post", "title": "A draft"}))
        .await;
    paths.push(format!("/api/v1/content/entries/{}", created(draft, "draft")));

    let body = json!({"type": "doc", "content": [
        {"type": "paragraph", "content": [{"type": "text", "text": "Before the upgrade."}]},
        {"type": "image", "attrs": {"mediaId": photo, "alt": "A photo"}},
    ]});
    let post = api
        .post(
            "/api/v1/content/entries",
            Some(token),
            json!({"kind": "post", "title": "Published", "body": body, "featured_media_id": photo,
                   "featured_media_alt": "A photo"}),
        )
        .await;
    let post = created(post, "post");
    let published = api
        .call(
            "POST",
            &format!("/api/v1/content/entries/{post}/publish"),
            Some(token),
            &[("if-match", "*")],
            Some(Body::Json(json!({}))),
        )
        .await;
    assert_eq!(published.status, 200, "publish: {}", published.body);
    paths.push(format!("/api/v1/content/entries/{post}"));

    let form = api
        .post(
            "/api/v1/forms",
            Some(token),
            json!({"key": "contact", "name": "Contact", "fields": [
                {"key": "email", "type": "email", "label": {"en": "Email"}, "required": true},
                {"key": "message", "type": "textarea", "label": {"en": "Message", "vi": "Lời nhắn"}},
            ]}),
        )
        .await;
    paths.push(format!("/api/v1/forms/{}", created(form, "form")));

    let list = api
        .post(
            "/api/v1/newsletter/lists",
            Some(token),
            json!({"key": "news", "name": {"en": "News"}}),
        )
        .await;
    paths.push(format!("/api/v1/newsletter/lists/{}", created(list, "newsletter list")));

    let webhook = api
        .post(
            "/api/v1/webhooks",
            Some(token),
            json!({"url": "https://example.com/hook", "events": ["content.entry_published"]}),
        )
        .await;
    paths.push(format!("/api/v1/webhooks/{}", created(webhook, "webhook")));

    // Module `menus` có từ 0.13.0.
    if version >= (0, 13, 0) {
        let menu = api
            .post(
                "/api/v1/menus",
                Some(token),
                json!({"key": "main", "items": [{"label": "Home", "url": "/"}]}),
            )
            .await;
        paths.push(format!("/api/v1/menus/{}", created(menu, "menu")));
    }
    (paths, post)
}

#[tokio::test]
async fn nang_cap_tu_ban_phat_hanh_truoc_giu_nguyen_du_lieu() {
    let Ok(image) = std::env::var(IMAGE_VARIABLE) else {
        eprintln!("skipped: set {IMAGE_VARIABLE} (cargo xtask upgrade-test does it)");
        return;
    };
    TestDatabase::run(async |test| {
        let directory = WorkDir::new();
        // SQLite: một file trong thư mục dữ liệu chung, đường dẫn tương đối như mặc định của
        // chương trình — container (`/var/lib/autoxide/data`) và máy (`<thư mục>/data`) cùng
        // thấy nó. Postgres: database trống của test.
        let database_url = match test.backend() {
            DbBackend::Sqlite => "sqlite://data/autoxide.sqlite".to_owned(),
            _ => test.url().to_owned(),
        };
        let port = free_port();
        let environment = environment(&database_url, port);
        let api = Api::new(port);

        // --- bản cũ ---
        let version = docker(&image, directory.path(), &environment, &["--version"], None, None);
        assert!(version.status.success(), "{}", text(&version));
        let old_version = parse_version(String::from_utf8_lossy(&version.stdout).trim());
        let migrated = docker(&image, directory.path(), &environment, &["migrate", "up"], None, None);
        assert!(migrated.status.success(), "old `migrate up`: {}", text(&migrated));
        let user = docker(
            &image,
            directory.path(),
            &environment,
            &["user", "create", "--email", EMAIL, "--name", "Admin", "--admin", "--password-stdin"],
            Some(PASSWORD),
            None,
        );
        assert!(user.status.success(), "old `user create`: {}", text(&user));
        let name = format!("autoxide-upgrade-{}", uuid::Uuid::now_v7());
        let started = docker(&image, directory.path(), &environment, &["serve"], None, Some(&name));
        assert!(started.status.success(), "old `serve`: {}", text(&started));
        let container = Container(name);
        let logs = || text(&Command::new("docker").args(["logs", &container.0]).output().unwrap());
        api.wait_ready(logs).await;

        let (access, refresh) = api.login().await;
        let key = api
            .post(
                "/api/v1/auth/api-keys",
                Some(&access),
                json!({"name": "agent", "permissions": ["content.read"], "current_password": PASSWORD}),
            )
            .await;
        assert_eq!(key.status, 201, "api key: {}", key.body);
        let api_key = key.body["data"]["key"].as_str().unwrap().to_owned();
        let (paths, post) = write_sample(&api, &access, old_version).await;
        let mut before = Vec::new();
        for path in &paths {
            let answer = api.get(path, &access).await;
            assert_eq!(answer.status, 200, "old GET {path}: {}", answer.body);
            before.push(answer.body);
        }

        // Dừng êm (SIGTERM) như khi triển khai bản mới.
        let stop = Command::new("docker").args(["stop", "--time", "30", &container.0]).output().unwrap();
        assert!(stop.status.success(), "{}", text(&stop));
        let stopped = Command::new("docker")
            .args(["inspect", "--format", "{{.State.ExitCode}}", &container.0])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&stopped.stdout).trim(), "0", "{}", logs());
        drop(container);

        // --- bản hiện tại, cùng database ---
        for arguments in [&["migrate", "up"][..], &["doctor"]] {
            let output = current(directory.path(), &environment, arguments);
            assert!(output.status.success(), "new `{}`: {}", arguments.join(" "), text(&output));
        }
        let server = Server(
            Command::new(env!("CARGO_BIN_EXE_autoxide"))
                .arg("serve")
                .current_dir(directory.path())
                .env_clear()
                .envs(environment.iter().map(|(key, value)| (key.as_str(), value.as_str())))
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        api.wait_ready(|| "the current `serve` did not answer /health/ready".to_owned()).await;

        // Mọi bản ghi đọc lại được, cùng trường, cùng giá trị.
        let mut problems = Vec::new();
        for (path, old) in paths.iter().zip(&before) {
            let answer = api.get(path, &access).await;
            if answer.status == 200 {
                contained(old, &answer.body, path, &mut problems);
            } else {
                problems.push(format!("{path}: answered {} after the upgrade: {}", answer.status, answer.body));
            }
        }
        assert!(problems.is_empty(), "{} difference(s):\n{}", problems.len(), problems.join("\n"));

        // Token cũ còn dùng được: access token, refresh token, API key, mật khẩu.
        let me = api.get("/api/v1/auth/me", &access).await;
        assert_eq!(me.status, 200, "old access token: {}", me.body);
        let refreshed = api.post("/api/v1/auth/refresh", None, json!({"refresh_token": refresh})).await;
        assert_eq!(refreshed.status, 200, "old refresh token: {}", refreshed.body);
        let listed = api.get("/api/v1/content/entries", &api_key).await;
        assert_eq!(listed.status, 200, "old API key: {}", listed.body);
        let (access, _) = api.login().await;

        // Vài lệnh ghi trên dữ liệu cũ và mới.
        let entry = format!("/api/v1/content/entries/{post}");
        let read = api.get(&entry, &access).await;
        let etag = read.etag.unwrap();
        let edited = api
            .call(
                "PATCH",
                &entry,
                Some(&access),
                &[("if-match", &etag)],
                Some(Body::Json(json!({"title": "Published, then upgraded"}))),
            )
            .await;
        assert_eq!(edited.status, 200, "edit: {}", edited.body);
        assert_eq!(edited.body["data"]["title"], "Published, then upgraded");
        api.upload_photo(&access, "after.png").await;
        let role = api
            .post("/api/v1/auth/roles", Some(&access), json!({"name": "reviewer", "permissions": ["content.read"]}))
            .await;
        assert_eq!(role.status, 201, "role: {}", role.body);
        drop(server);

        let task = current(directory.path(), &environment, &["task", "media.refresh_variants"]);
        assert!(task.status.success(), "new `task media.refresh_variants`: {}", text(&task));
    })
    .await
    .unwrap();
}

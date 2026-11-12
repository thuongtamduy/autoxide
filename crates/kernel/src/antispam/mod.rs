//! Chống spam cho dữ liệu người lạ gửi lên (form, đăng ký bản tin, bình luận) — D-113.
//!
//! Module lấy [`SpamGuard`] lúc khởi động (`context.require::<SpamGuard>()`) và chạy **hai pha**:
//!
//! 1. **Trước** transaction ghi: [`SpamGuard::check`] — trường bẫy, token form do máy chủ ký
//!    (tuổi tối thiểu / tối đa), đề (`ChallengeVerifier`, mặc định không có), giới hạn theo IP
//!    băm, theo email, theo người dùng ([`Limits`], bảng `<module>_throttle`), trùng nội dung
//!    (module tự dò, báo qua [`Submission::duplicate`]), bộ phân loại nội dung
//!    (`ContentClassifier`, mặc định không có). Mọi việc gọi mạng nằm ở pha này.
//! 2. **Trong** transaction của module, sau khi tạo bản ghi: [`Checked::claim`] chèn nonce của
//!    token vào `<module>_antispam_nonces`. [`Claim::Replay`] nghĩa là bài này đã gửi rồi:
//!    rollback và trả lại kết quả lần đầu.
//!
//! Kết luận ([`Verdict`]) cho người lạ không bao giờ lộ cho bot: bẫy, quá nhiều lần → `Discard`
//! (trả lời như đã nhận, không lưu gì); quá nhanh, trùng, bị phân loại là spam → `Quarantine`
//! (lưu, giữ lại chờ duyệt, người gửi không biết). Chỉ token hỏng (người thật tải lại form là
//! xong), đề sai, và nhà cung cấp đề lỗi khi `AUTOXIDE_ANTISPAM_FAIL_MODE=reject` mới là lỗi.
//!
//! Request có `Principal` (website gọi bằng API key, MCP) bỏ qua bẫy, token và đề, nhưng vẫn
//! bị giới hạn theo người dùng (vượt thì 429 `antispam.rate_limited`).

mod altcha;
mod nonce;
mod token;

use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, SubsecRound, Utc};
use sea_orm::ConnectionTrait;

use crate::access::Principal;
use crate::config::{ConfigReader, Invalid, Setting, parse_in_range};
use crate::error::{Error, FieldError};
use crate::http::ClientInfo;
use crate::secret::SecretKey;
use crate::throttle::{Outcome, Throttle};

pub use altcha::Altcha;
pub use nonce::{Claim, antispam_migration};

pub(crate) mod module;

pub(crate) const CHALLENGE: Setting = Setting::new(
    "AUTOXIDE_ANTISPAM_CHALLENGE",
    "Proof that a person sends a public form: `none` (default; the other layers — trap field, \
     signed form token, rate limits — still apply) or `altcha` (a proof-of-work the browser \
     solves with the ALTCHA widget, self-hosted, no third-party account).",
)
.with_default("none");

pub(crate) const ALTCHA_COST: Setting = Setting::new(
    "AUTOXIDE_ANTISPAM_ALTCHA_COST",
    "With AUTOXIDE_ANTISPAM_CHALLENGE=altcha: rounds of SHA-256 per guess (10 to 100000); the \
     browser makes 32 to 64 guesses. Higher costs bots more and slow phones too: aim for under \
     a second on a mid-range phone (not measured yet; adjust after trying).",
)
.with_default("500");

pub(crate) const MIN_FILL_SECONDS: Setting = Setting::new(
    "AUTOXIDE_ANTISPAM_MIN_FILL_SECONDS",
    "A public form sent sooner than this many seconds after its form token was issued is held \
     for review (0 to 600). People take longer than bots.",
)
.with_default("3");

pub(crate) const TOKEN_MAX_AGE_MINUTES: Setting = Setting::new(
    "AUTOXIDE_ANTISPAM_TOKEN_MAX_AGE_MINUTES",
    "How long a form token stays valid, in minutes (1 to 1440). The website fetches a token \
     when it shows the form, never into cached HTML.",
)
.with_default("120");

pub(crate) const FAIL_MODE: Setting = Setting::new(
    "AUTOXIDE_ANTISPAM_FAIL_MODE",
    "What to do with a submission when a challenge provider or content classifier cannot be \
     reached: `quarantine` (accept it and hold it for review) or `reject` (answer 503).",
)
.with_default("quarantine");

/// Khi nhà cung cấp đề hay bộ phân loại lỗi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailMode {
    Quarantine,
    Reject,
}

/// Loại đề chống bot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChallengeKind {
    None,
    /// ALTCHA v2, `cost` vòng SHA-256 mỗi lần thử.
    Altcha {
        cost: u32,
    },
}

/// Cấu hình chống spam.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AntispamConfig {
    pub challenge: ChallengeKind,
    pub min_fill: StdDuration,
    pub token_max_age: StdDuration,
    pub fail_mode: FailMode,
}

impl Default for AntispamConfig {
    fn default() -> Self {
        AntispamConfig {
            challenge: ChallengeKind::None,
            min_fill: StdDuration::from_secs(3),
            token_max_age: StdDuration::from_secs(120 * 60),
            fail_mode: FailMode::Quarantine,
        }
    }
}

impl AntispamConfig {
    pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<Self, Invalid> {
        let kind = reader.get_with(&CHALLENGE, |raw| match raw {
            "none" | "altcha" => Ok(raw.to_owned()),
            _ => Err("must be `none` or `altcha`".to_owned()),
        });
        let cost = reader.get_with(&ALTCHA_COST, |raw| parse_in_range(raw, 10u32, 100_000));
        let min_fill = reader.get_with(&MIN_FILL_SECONDS, |raw| parse_in_range(raw, 0u64, 600));
        let max_age =
            reader.get_with(&TOKEN_MAX_AGE_MINUTES, |raw| parse_in_range(raw, 1u64, 1440));
        let fail_mode = reader.get_with(&FAIL_MODE, |raw| match raw {
            "quarantine" => Ok(FailMode::Quarantine),
            "reject" => Ok(FailMode::Reject),
            _ => Err("must be `quarantine` or `reject`".to_owned()),
        });
        let (kind, cost) = (kind?, cost?);
        Ok(AntispamConfig {
            challenge: if kind == "altcha" {
                ChallengeKind::Altcha { cost }
            } else {
                ChallengeKind::None
            },
            min_fill: StdDuration::from_secs(min_fill?),
            token_max_age: StdDuration::from_secs(max_age? * 60),
            fail_mode: fail_mode?,
        })
    }
}

/// Kết quả của một đề (proof-of-work, CAPTCHA…).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChallengeOutcome {
    Passed,
    /// Không gửi lời giải mà đề là bắt buộc.
    Missing,
    Failed,
    /// Nhà cung cấp không trả lời được.
    Unavailable,
}

/// Chỗ cắm đề chống bot. Mặc định không có đề ([`NoChallenge`]).
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait ChallengeVerifier: Send + Sync {
    /// Tham số đề gửi cho trình duyệt cùng token form; `None` khi không có đề.
    /// `binding` (nonce của token form) gắn đề với đúng token đó, để một lời giải không dùng lại
    /// được.
    async fn challenge(
        &self,
        scope: &str,
        binding: &str,
    ) -> Result<Option<serde_json::Value>, Error>;
    /// Kiểm lời giải `solution` của đề cho `scope` và token form có nonce `binding`.
    async fn verify(
        &self,
        scope: &str,
        binding: &str,
        solution: Option<&str>,
    ) -> Result<ChallengeOutcome, Error>;
}

/// Không có đề: mọi bài đều qua bước này.
pub struct NoChallenge;

#[async_trait::async_trait]
impl ChallengeVerifier for NoChallenge {
    async fn challenge(&self, _: &str, _: &str) -> Result<Option<serde_json::Value>, Error> {
        Ok(None)
    }

    async fn verify(
        &self,
        _scope: &str,
        _binding: &str,
        _solution: Option<&str>,
    ) -> Result<ChallengeOutcome, Error> {
        Ok(ChallengeOutcome::Passed)
    }
}

/// Kết luận của bộ phân loại nội dung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Classification {
    Ham,
    Spam,
    /// Bộ phân loại không trả lời được.
    Unavailable,
}

/// Chỗ cắm bộ phân loại nội dung (vd Akismet). Mặc định không có ([`NoClassifier`]).
#[async_trait::async_trait]
pub trait ContentClassifier: Send + Sync {
    async fn classify(
        &self,
        scope: &str,
        content: &str,
        email: Option<&str>,
        client: &ClientInfo,
    ) -> Result<Classification, Error>;
}

/// Không phân loại: mọi nội dung là "không phải spam".
pub struct NoClassifier;

#[async_trait::async_trait]
impl ContentClassifier for NoClassifier {
    async fn classify(
        &self,
        _: &str,
        _: &str,
        _: Option<&str>,
        _: &ClientInfo,
    ) -> Result<Classification, Error> {
        Ok(Classification::Ham)
    }
}

/// Một bài gửi cần xét.
#[derive(Debug, Clone, Copy)]
pub struct Submission<'a> {
    /// Phạm vi: form nào (vd `forms:contact`, `newsletter:subscribe`); khớp với phạm vi của
    /// token form.
    pub scope: &'a str,
    /// Người gọi đã đăng nhập (API key của website, MCP), nếu có.
    pub principal: Option<&'a Principal>,
    pub client: &'a ClientInfo,
    /// Giá trị trường bẫy (người không thấy nên để trống).
    pub honeypot: Option<&'a str>,
    pub form_token: Option<&'a str>,
    /// Lời giải đề, nếu có đề.
    pub challenge: Option<&'a str>,
    /// Email người gửi tự khai (giới hạn theo email).
    pub email: Option<&'a str>,
    /// Nội dung chữ (cho bộ phân loại).
    pub content: Option<&'a str>,
    /// Module đã thấy cùng nội dung này gần đây (tự dò trên bảng của mình bằng
    /// [`content_hash`]).
    pub duplicate: bool,
}

/// Giới hạn số lần, trên bảng `<module>_throttle` của module. Thiếu thì không giới hạn kiểu đó.
#[derive(Debug, Clone, Default)]
pub struct Limits {
    /// Theo khối IP băm (của người lạ).
    pub ip: Option<Throttle>,
    /// Theo email người gửi tự khai.
    pub email: Option<Throttle>,
    /// Theo người dùng đã đăng nhập.
    pub user: Option<Throttle>,
}

/// Kết luận cho một bài.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Nhận.
    Accept,
    /// Nhận nhưng giữ lại chờ duyệt; người gửi được trả lời như bài thường.
    Quarantine { reasons: Vec<&'static str> },
    /// Bỏ đi, không lưu gì; người gửi vẫn được trả lời như bài thường.
    Discard { reason: &'static str },
}

/// Kết quả pha một: kết luận, và nonce để chèn ở pha hai.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    pub verdict: Verdict,
    nonce: Option<String>,
    expires_at: DateTime<Utc>,
}

impl Checked {
    /// Pha hai, TRONG transaction của module (sau khi tạo bản ghi): chèn nonce kèm `result` (vd
    /// ID bản ghi vừa tạo). Không có nonce (người gọi đã đăng nhập — họ dùng
    /// `Idempotency-Key`) thì luôn [`Claim::New`].
    pub async fn claim<C: ConnectionTrait>(
        &self,
        connection: &C,
        module: &str,
        result: Option<&str>,
    ) -> Result<Claim, Error> {
        match &self.nonce {
            Some(nonce) => nonce::claim(connection, module, nonce, result, self.expires_at).await,
            None => Ok(Claim::New),
        }
    }
}

/// Bộ chống spam dùng chung. Clone rẻ.
#[derive(Clone)]
pub struct SpamGuard {
    secret: SecretKey,
    config: AntispamConfig,
    challenge: Arc<dyn ChallengeVerifier>,
    classifier: Arc<dyn ContentClassifier>,
}

impl std::fmt::Debug for SpamGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpamGuard").field("config", &self.config).finish_non_exhaustive()
    }
}

/// Token form vừa phát cho website.
#[derive(Debug, Clone, PartialEq)]
pub struct IssuedToken {
    pub token: String,
    pub expires_at: DateTime<Utc>,
    /// Gửi sớm hơn lúc này thì bài bị giữ lại.
    pub submit_after: DateTime<Utc>,
    /// Tham số đề, nếu có đề.
    pub challenge: Option<serde_json::Value>,
}

impl SpamGuard {
    pub fn new(secret: SecretKey, config: AntispamConfig) -> Self {
        // Đề theo cấu hình.
        let challenge: Arc<dyn ChallengeVerifier> = match config.challenge {
            ChallengeKind::None => Arc::new(NoChallenge),
            ChallengeKind::Altcha { cost } => Arc::new(Altcha::new(secret.clone(), cost)),
        };
        SpamGuard { secret, config, challenge, classifier: Arc::new(NoClassifier) }
    }

    /// Thay bộ đề.
    pub fn with_challenge(mut self, challenge: Arc<dyn ChallengeVerifier>) -> Self {
        self.challenge = challenge;
        self
    }

    /// Thay bộ phân loại nội dung.
    pub fn with_classifier(mut self, classifier: Arc<dyn ContentClassifier>) -> Self {
        self.classifier = classifier;
        self
    }

    /// Phát token form cho `scope`.
    pub async fn issue(&self, scope: &str) -> Result<IssuedToken, Error> {
        check_scope(scope)?;
        let now = Utc::now().trunc_subsecs(0);
        let mut nonce = [0u8; 16];
        getrandom::fill(&mut nonce)
            .map_err(|error| Error::internal(format!("random number generator: {error}")))?;
        let key = self.secret.derive(token::PURPOSE).await?;
        let binding = token::encode_nonce(&nonce);
        Ok(IssuedToken {
            token: token::sign(&key, scope, now, &nonce)?,
            expires_at: now + self.max_age(),
            submit_after: now + self.min_fill(),
            challenge: self.challenge.challenge(scope, &binding).await?,
        })
    }

    /// Pha một: xét `submission` trước transaction ghi.
    pub async fn check(
        &self,
        submission: &Submission<'_>,
        limits: &Limits,
    ) -> Result<Checked, Error> {
        let now = Utc::now();
        let mut reasons = Vec::new();
        let mut nonce = None;
        let mut expires_at = now + self.max_age();
        match submission.principal {
            Some(principal) => {
                if let Some(throttle) = &limits.user
                    && let Outcome::Limited { retry_after } =
                        throttle.hit(&format!("user:{}", principal.user_id())).await?
                {
                    return Err(rate_limited(retry_after));
                }
            }
            None => {
                if submission.honeypot.is_some_and(|value| !value.trim().is_empty()) {
                    return Ok(discard("honeypot", now));
                }
                let raw = submission
                    .form_token
                    .filter(|token| !token.trim().is_empty())
                    .ok_or_else(token_missing)?;
                let key = self.secret.derive(token::PURPOSE).await?;
                let form = token::open(&key, raw)
                    .filter(|form| form.scope == submission.scope)
                    .ok_or_else(token_invalid)?;
                if form.issued_at + self.max_age() < now {
                    return Err(token_expired());
                }
                if form.issued_at + self.min_fill() > now {
                    reasons.push("too_fast");
                }
                expires_at = form.issued_at + self.max_age() + Duration::hours(1);
                let outcome = self
                    .challenge
                    .verify(submission.scope, &form.nonce, submission.challenge)
                    .await?;
                nonce = Some(form.nonce);
                match outcome {
                    ChallengeOutcome::Passed => {}
                    ChallengeOutcome::Missing => return Err(challenge_required()),
                    ChallengeOutcome::Failed => return Err(challenge_failed()),
                    ChallengeOutcome::Unavailable => match self.config.fail_mode {
                        FailMode::Quarantine => reasons.push("challenge_unavailable"),
                        FailMode::Reject => return Err(verifier_unavailable()),
                    },
                }
                if let (Some(throttle), Some(ip)) = (&limits.ip, submission.client.ip) {
                    let bucket = crate::ip_hash::bucket(&self.secret, ip, now).await?;
                    if !throttle
                        .hit(&format!("ip:{}:{bucket}", submission.scope))
                        .await?
                        .is_allowed()
                    {
                        return Ok(Checked {
                            verdict: Verdict::Discard { reason: "rate_limited" },
                            nonce: None,
                            expires_at,
                        });
                    }
                }
                if let (Some(throttle), Some(email)) = (&limits.email, submission.email) {
                    let key = format!("email:{}:{}", submission.scope, email.trim().to_lowercase());
                    if !throttle.hit(&key).await?.is_allowed() {
                        return Ok(Checked {
                            verdict: Verdict::Discard { reason: "rate_limited" },
                            nonce: None,
                            expires_at,
                        });
                    }
                }
            }
        }
        if submission.duplicate {
            reasons.push("duplicate");
        }
        if let Some(content) = submission.content.filter(|content| !content.trim().is_empty()) {
            match self
                .classifier
                .classify(submission.scope, content, submission.email, submission.client)
                .await?
            {
                Classification::Ham => {}
                Classification::Spam => reasons.push("classifier"),
                Classification::Unavailable => match self.config.fail_mode {
                    FailMode::Quarantine => reasons.push("classifier_unavailable"),
                    FailMode::Reject => return Err(verifier_unavailable()),
                },
            }
        }
        let verdict =
            if reasons.is_empty() { Verdict::Accept } else { Verdict::Quarantine { reasons } };
        Ok(Checked { verdict, nonce, expires_at })
    }

    /// Xoá nonce đã hết hạn của `module` (job dọn dẹp của module). Trả số dòng đã xoá.
    pub async fn purge_nonces<C: ConnectionTrait>(
        &self,
        connection: &C,
        module: &str,
    ) -> Result<u64, Error> {
        nonce::purge(connection, module, Utc::now()).await
    }

    fn max_age(&self) -> Duration {
        Duration::from_std(self.config.token_max_age).unwrap_or_else(|_| Duration::hours(2))
    }

    fn min_fill(&self) -> Duration {
        Duration::from_std(self.config.min_fill).unwrap_or_else(|_| Duration::seconds(3))
    }
}

fn discard(reason: &'static str, now: DateTime<Utc>) -> Checked {
    Checked { verdict: Verdict::Discard { reason }, nonce: None, expires_at: now }
}

/// Băm nội dung để module dò trùng trên bảng của mình: bỏ khoảng trắng thừa, viết thường, rồi
/// SHA-256.
pub fn content_hash(text: &str) -> String {
    let normalized: Vec<String> = text.split_whitespace().map(str::to_lowercase).collect();
    crate::secret::sha256(&normalized.join(" "))
}

/// Phạm vi: 1–64 ký tự chữ thường, số, `_`, `-`, `.`, `:`.
fn check_scope(scope: &str) -> Result<(), Error> {
    let valid = (1..=64).contains(&scope.len())
        && scope.chars().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '.' | ':')
        });
    if valid {
        Ok(())
    } else {
        Err(Error::validation(vec![FieldError::new(
            "scope",
            "invalid",
            "must be 1 to 64 lower-case letters, digits, `_`, `-`, `.` or `:`, such as `forms:contact`",
        )]))
    }
}

fn token_missing() -> Error {
    Error::bad_request(
        "antispam.token_missing",
        "This form needs a form token. Get one from `GET /api/v1/antispam/token?scope=…` when you \
         show the form, and send it with the form.",
    )
}

fn token_invalid() -> Error {
    Error::bad_request(
        "antispam.token_invalid",
        "The form token is not valid for this form. Reload the form to get a new one.",
    )
}

fn token_expired() -> Error {
    Error::bad_request(
        "antispam.token_expired",
        "The form token has expired. Reload the form and send it again.",
    )
}

fn challenge_required() -> Error {
    Error::bad_request(
        "antispam.challenge_required",
        "This form needs the solution of its challenge. Reload the form.",
    )
}

fn challenge_failed() -> Error {
    Error::bad_request(
        "antispam.challenge_failed",
        "The challenge solution is wrong or expired. Reload the form and try again.",
    )
}

fn verifier_unavailable() -> Error {
    Error::unavailable(
        "antispam.verifier_unavailable",
        "The anti-spam check cannot run right now. Try again in a few minutes.",
    )
}

fn rate_limited(seconds: u64) -> Error {
    Error::too_many_requests(
        "antispam.rate_limited",
        "Too many submissions. Wait {seconds} seconds, then try again.",
    )
    .with_param("seconds", seconds)
    .with_retry_after(seconds)
}

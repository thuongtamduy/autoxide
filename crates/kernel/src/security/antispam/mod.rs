//! Anti-spam for data submitted by strangers (forms, newsletter sign-ups, comments) — D-119.
//!
//! A module gets the [`SpamGuard`] at startup (`context.require::<SpamGuard>()`) and runs **two phases**:
//!
//! 1. **Before** the write transaction: [`SpamGuard::check`] — honeypot field, server-signed form token
//!    (minimum / maximum age), challenge (`ChallengeVerifier`, none by default), limits per hashed
//!    IP, per email, per user ([`Limits`], table `<module>_throttle`), duplicate content
//!    (the module detects it itself and reports via [`Submission::duplicate`]), content classifier
//!    (`ContentClassifier`, none by default). Every network call happens in this phase.
//! 2. **Inside** the module's transaction, after creating the record: [`Checked::claim`] inserts the token's
//!    nonce into `<module>_antispam_nonces`. [`Claim::Replay`] means this submission was already sent:
//!    roll back and return the result of the first time.
//!
//! The conclusion ([`Verdict`]) for a stranger is never revealed to a bot: honeypot, too many attempts →
//! `Discard` (answer as if received, store nothing); too fast, duplicate, classified as spam → `Quarantine`
//! (stored, held for review, the sender does not know). Only a broken token (a real person reloads the form
//! and is done), a wrong challenge, and a challenge provider failing when
//! `AUTOXIDE_ANTISPAM_FAIL_MODE=reject` are errors.
//!
//! A request with a `Principal` (the website calling with an API key, MCP) skips the honeypot, token and
//! challenge, but is still limited per user (beyond the limit: 429 `antispam.rate_limited`).

mod altcha;
mod nonce;
mod token;

use crate::time::Timestamp;
use jiff::SignedDuration;
use sea_orm::ConnectionTrait;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use crate::access::Principal;
use crate::config::{ConfigReader, Invalid, Setting, parse_in_range};
use crate::error::{Error, FieldError};
use crate::http::ClientInfo;
use crate::secret::SecretKey;
use crate::tenancy::TenantContext;
use crate::throttle::{Outcome, Throttle};

pub use altcha::Altcha;
pub use nonce::{Claim, antispam_migration, tenant_table};

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

/// When the challenge provider or the classifier fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FailMode {
    Quarantine,
    Reject,
}

/// Kind of anti-bot challenge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ChallengeKind {
    None,
    /// ALTCHA v2, `cost` SHA-256 rounds per attempt.
    Altcha {
        cost: u32,
    },
}

/// Anti-spam configuration. Built with `AntispamConfig::default()` followed by the per-field setters.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
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
    /// Sets the challenge visitors must solve.
    pub const fn challenge(mut self, challenge: ChallengeKind) -> Self {
        self.challenge = challenge;
        self
    }

    /// Sets the shortest time between getting a form token and sending the form.
    pub const fn min_fill(mut self, min_fill: StdDuration) -> Self {
        self.min_fill = min_fill;
        self
    }

    /// Sets how long a form token stays valid.
    pub const fn token_max_age(mut self, token_max_age: StdDuration) -> Self {
        self.token_max_age = token_max_age;
        self
    }

    /// Sets what happens when a check cannot run.
    pub const fn fail_mode(mut self, fail_mode: FailMode) -> Self {
        self.fail_mode = fail_mode;
        self
    }

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

/// Result of a challenge (proof-of-work, CAPTCHA…).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ChallengeOutcome {
    Passed,
    /// No solution was sent although the challenge is required.
    Missing,
    Failed,
    /// The provider could not be reached.
    Unavailable,
}

/// Plug-in point for an anti-bot challenge. There is no challenge by default ([`NoChallenge`]).
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait ChallengeVerifier: Send + Sync {
    /// Challenge parameters sent to the browser along with the form token; `None` when there is no challenge.
    /// `binding` (the form token's nonce) ties the challenge to exactly that token, so one solution cannot be
    /// reused.
    async fn challenge(
        &self,
        scope: &str,
        binding: &str,
    ) -> Result<Option<serde_json::Value>, Error>;
    /// Verifies the challenge `solution` for `scope` and the form token with nonce `binding`.
    async fn verify(
        &self,
        scope: &str,
        binding: &str,
        solution: Option<&str>,
    ) -> Result<ChallengeOutcome, Error>;
}

/// No challenge: every submission passes this step.
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

/// Conclusion of the content classifier.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Classification {
    Ham,
    Spam,
    /// The classifier could not be reached.
    Unavailable,
}

/// Plug-in point for a content classifier (e.g. Akismet). None by default ([`NoClassifier`]).
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

/// No classification: all content is "not spam".
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

/// A submission to evaluate. Built with [`Submission::new`].
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct Submission<'a> {
    /// Scope: which form (e.g. `forms:contact`, `newsletter:subscribe`); matches the scope of the
    /// form token.
    pub scope: &'a str,
    /// The signed-in caller (the website's API key, MCP), if any.
    pub principal: Option<&'a Principal>,
    pub client: &'a ClientInfo,
    /// Value of the honeypot field (people do not see it, so it should stay empty).
    pub honeypot: Option<&'a str>,
    pub form_token: Option<&'a str>,
    /// The challenge solution, if there is a challenge.
    pub challenge: Option<&'a str>,
    /// Email the sender gave themselves (limit per email).
    pub email: Option<&'a str>,
    /// Text content (for the classifier).
    pub content: Option<&'a str>,
    /// The module has seen this same content recently (it detects this itself on its own table with
    /// [`content_hash`]).
    pub duplicate: bool,
}

impl<'a> Submission<'a> {
    /// A submission to form `scope` from `client`, with nothing else filled in yet.
    pub const fn new(scope: &'a str, client: &'a ClientInfo) -> Self {
        Submission {
            scope,
            principal: None,
            client,
            honeypot: None,
            form_token: None,
            challenge: None,
            email: None,
            content: None,
            duplicate: false,
        }
    }

    /// Sets the signed-in caller, if any.
    pub const fn principal(mut self, principal: Option<&'a Principal>) -> Self {
        self.principal = principal;
        self
    }

    /// Sets the value of the trap field.
    pub const fn honeypot(mut self, honeypot: Option<&'a str>) -> Self {
        self.honeypot = honeypot;
        self
    }

    /// Sets the form token.
    pub const fn form_token(mut self, form_token: Option<&'a str>) -> Self {
        self.form_token = form_token;
        self
    }

    /// Sets the answer to the challenge.
    pub const fn challenge(mut self, challenge: Option<&'a str>) -> Self {
        self.challenge = challenge;
        self
    }

    /// Sets the email the sender gave.
    pub const fn email(mut self, email: Option<&'a str>) -> Self {
        self.email = email;
        self
    }

    /// Sets the text content.
    pub const fn content(mut self, content: Option<&'a str>) -> Self {
        self.content = content;
        self
    }

    /// Sets whether the module saw the same content recently.
    pub const fn duplicate(mut self, duplicate: bool) -> Self {
        self.duplicate = duplicate;
        self
    }
}

/// Rate limits, on the module's table `<module>_throttle`. If missing, that kind of limit does not apply.
/// Built with `Limits::default()` followed by the per-field setters.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Limits {
    /// Per hashed IP block (of a stranger).
    pub ip: Option<Throttle>,
    /// Per email the sender gave themselves.
    pub email: Option<Throttle>,
    /// Per signed-in user.
    pub user: Option<Throttle>,
}

impl Limits {
    /// Sets the limit per hashed IP block.
    pub fn ip(mut self, ip: Option<Throttle>) -> Self {
        self.ip = ip;
        self
    }

    /// Sets the limit per email the sender gave.
    pub fn email(mut self, email: Option<Throttle>) -> Self {
        self.email = email;
        self
    }

    /// Sets the limit per signed-in user.
    pub fn user(mut self, user: Option<Throttle>) -> Self {
        self.user = user;
        self
    }
}

/// Conclusion for a submission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Accept.
    Accept,
    /// Accept but hold for review; the sender is answered as with a normal submission.
    Quarantine { reasons: Vec<&'static str> },
    /// Drop, store nothing; the sender is still answered as with a normal submission.
    Discard { reason: &'static str },
}

/// Result of phase one: the conclusion, and the nonce to insert in phase two.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    pub verdict: Verdict,
    nonce: Option<String>,
    expires_at: Timestamp,
}

impl Checked {
    /// Phase two, INSIDE the module's transaction (after creating the record): inserts the nonce with
    /// `result` (e.g. the ID of the record just created). With no nonce (a signed-in caller — they use
    /// `Idempotency-Key`) it is always [`Claim::New`].
    pub async fn claim<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        module: &str,
        result: Option<&str>,
    ) -> Result<Claim, Error> {
        match &self.nonce {
            Some(nonce) => {
                nonce::claim(connection, tenant, module, nonce, result, self.expires_at).await
            }
            None => Ok(Claim::New),
        }
    }
}

/// The shared anti-spam guard. Cheap to clone.
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

/// A form token just issued to the website.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct IssuedToken {
    pub token: String,
    pub expires_at: Timestamp,
    /// A submission sent earlier than this is held back.
    pub submit_after: Timestamp,
    /// Challenge parameters, if there is a challenge.
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

    /// Replaces the challenge.
    pub fn with_challenge(mut self, challenge: Arc<dyn ChallengeVerifier>) -> Self {
        self.challenge = challenge;
        self
    }

    /// Replaces the content classifier.
    pub fn with_classifier(mut self, classifier: Arc<dyn ContentClassifier>) -> Self {
        self.classifier = classifier;
        self
    }

    /// Issues a form token for `scope` of tenant `tenant` (signed with the tenant's key: used with another
    /// tenant it is like a wrong token).
    pub async fn issue(&self, tenant: &TenantContext, scope: &str) -> Result<IssuedToken, Error> {
        check_scope(scope)?;
        let now = Timestamp::now().truncate(jiff::Unit::Second);
        let mut nonce = [0u8; 16];
        getrandom::fill(&mut nonce)
            .map_err(|error| Error::internal(format!("random number generator: {error}")))?;
        let key = self.secret.derive_for(tenant.id(), token::PURPOSE).await?;
        let binding = token::encode_nonce(&nonce);
        Ok(IssuedToken {
            token: token::sign(&key, scope, now, &nonce)?,
            expires_at: now + self.max_age(),
            submit_after: now + self.min_fill(),
            challenge: self.challenge.challenge(scope, &binding).await?,
        })
    }

    /// Phase one: evaluates `submission` sent to tenant `tenant`, before the write transaction.
    pub async fn check(
        &self,
        tenant: &TenantContext,
        submission: &Submission<'_>,
        limits: &Limits,
    ) -> Result<Checked, Error> {
        let now = Timestamp::now();
        let mut reasons = Vec::new();
        let mut nonce = None;
        let mut expires_at = now + self.max_age();
        match submission.principal {
            Some(principal) => {
                if let Some(throttle) = &limits.user
                    && let Outcome::Limited { retry_after } =
                        throttle.hit(tenant, &format!("user:{}", principal.user_id())).await?
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
                let key = self.secret.derive_for(tenant.id(), token::PURPOSE).await?;
                let form = token::open(&key, raw)
                    .filter(|form| form.scope == submission.scope)
                    .ok_or_else(token_invalid)?;
                if form.issued_at + self.max_age() < now {
                    return Err(token_expired());
                }
                // Token đã dùng cho một bài đã nhận (gửi lại): pha hai sẽ trả lại kết quả cũ. Dừng
                // ở đây, không đếm giới hạn theo IP / email, không gọi bộ phân loại — nếu không,
                // một token gửi lại nhiều lần khoá được email của người khác (D-150).
                let owner = [&limits.email, &limits.ip, &limits.user]
                    .into_iter()
                    .flatten()
                    .find_map(Throttle::owner);
                if let Some(module) = owner
                    && nonce::exists(tenant.database().read(), tenant, module, &form.nonce).await?
                {
                    return Ok(Checked {
                        verdict: Verdict::Accept,
                        nonce: Some(form.nonce),
                        expires_at: form.issued_at + self.max_age() + SignedDuration::from_hours(1),
                    });
                }
                if form.issued_at + self.min_fill() > now {
                    reasons.push("too_fast");
                }
                expires_at = form.issued_at + self.max_age() + SignedDuration::from_hours(1);
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
                    let bucket =
                        crate::ip_hash::bucket(&self.secret, submission.scope, ip, now).await?;
                    if !throttle
                        // address-key: `bucket` là dấu của khối /24 hay /64 (`ip_hash::bucket`).
                        .hit(tenant, &format!("ip:{}:{bucket}", submission.scope))
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
                    if !throttle.hit(tenant, &key).await?.is_allowed() {
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

    /// Deletes expired nonces of `module` (the module's cleanup job). Returns the number of rows deleted.
    pub async fn purge_nonces<C: ConnectionTrait>(
        &self,
        connection: &C,
        module: &str,
    ) -> Result<u64, Error> {
        nonce::purge(connection, module, Timestamp::now()).await
    }

    fn max_age(&self) -> SignedDuration {
        SignedDuration::try_from(self.config.token_max_age)
            .unwrap_or_else(|_| SignedDuration::from_hours(2))
    }

    fn min_fill(&self) -> SignedDuration {
        SignedDuration::try_from(self.config.min_fill)
            .unwrap_or_else(|_| SignedDuration::from_secs(3))
    }
}

fn discard(reason: &'static str, now: Timestamp) -> Checked {
    Checked { verdict: Verdict::Discard { reason }, nonce: None, expires_at: now }
}

/// Hashes content so a module can detect duplicates on its own table: strips extra whitespace, lowercases, then
/// SHA-256.
pub fn content_hash(text: &str) -> String {
    let normalized: Vec<String> = text.split_whitespace().map(str::to_lowercase).collect();
    crate::secret::sha256(&normalized.join(" "))
}

/// Scope: 1–64 characters of lowercase letters, digits, `_`, `-`, `.`, `:`.
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

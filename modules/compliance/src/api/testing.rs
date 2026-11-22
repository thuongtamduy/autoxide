//! Sổ đồng ý giả dùng chung cho test của module khác (feature `testing`, chỉ khai ở
//! `[dev-dependencies]`; quy ước D-20261122-module-test-fakes).
//!
//! Giống `compliance` thật ở chỗ module khác dựa vào:
//! - mỗi (người, khoá) một dòng, trạng thái `granted` / `withdrawn`; `grant` gọi lại cùng `source_ref`
//!   không ghi thêm, trả trạng thái hiện tại;
//! - `is_granted`: có đồng ý ở đúng khoá hay khoá rộng hơn (`scope` / `channel` rỗng bao các con), và
//!   không có rút lại ở khoá đó hay khoá rộng hơn;
//! - đồng ý phải có câu chữ và phiên bản câu chữ.
//!
//! Không phân biệt khách (`TenantId`): mỗi test một sổ.
//!
//! ```ignore
//! let consents = Arc::new(FakeConsentLedger::new());
//! consents.allow(ConsentSubject::User(user), ConsentKey::new("marketing", "", ""));
//! let app = TestApp::builder(test).provide::<dyn ConsentLedger>(consents.clone())…;
//! ```

use std::sync::{Mutex, MutexGuard, PoisonError};

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantId;
use autoxide_kernel::time::Timestamp;
use uuid::Uuid;

use super::{Consent, ConsentEvidence, ConsentKey, ConsentLedger, ConsentSubject};

const GRANTED: &str = "granted";
const WITHDRAWN: &str = "withdrawn";

/// Một dòng của sổ giả.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsentRecord {
    pub subject: ConsentSubject,
    pub consent: Consent,
}

#[derive(Default)]
struct State {
    records: Vec<ConsentRecord>,
    /// `source_ref` đã gặp, theo thứ tự (gọi lại không thêm).
    source_refs: Vec<String>,
    /// Sổ "hỏng": mọi lời gọi trả lỗi.
    failing: bool,
}

/// Sổ đồng ý giả, giữ mọi thứ trong bộ nhớ. Công bố vào app bằng
/// `TestApp::builder(…).provide::<dyn ConsentLedger>(consents.clone())`.
#[derive(Default)]
pub struct FakeConsentLedger {
    state: Mutex<State>,
}

// Khoá `Mutex` mà không `unwrap`: test khác panic giữa chừng thì vẫn đọc tiếp dữ liệu bên trong.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// `broad` bao `key`: cùng mục đích, phạm vi và kênh rỗng hay bằng nhau.
fn covers(broad: &Consent, key: &ConsentKey) -> bool {
    broad.purpose == key.purpose
        && (broad.scope.is_empty() || broad.scope == key.scope)
        && (broad.channel.is_empty() || broad.channel == key.channel)
}

fn subject_value(subject: &ConsentSubject) -> Option<String> {
    match subject {
        ConsentSubject::User(_) => None,
        ConsentSubject::Email(value) | ConsentSubject::Phone(value) => Some(value.clone()),
    }
}

impl State {
    fn set(&mut self, subject: &ConsentSubject, key: &ConsentKey, state: &str) -> Consent {
        let now = Timestamp::now();
        let found = self.records.iter_mut().find(|record| {
            record.subject == *subject
                && record.consent.purpose == key.purpose
                && record.consent.scope == key.scope
                && record.consent.channel == key.channel
        });
        let record = match found {
            Some(record) => record,
            None => {
                self.records.push(ConsentRecord {
                    subject: subject.clone(),
                    consent: Consent {
                        id: Uuid::now_v7(),
                        purpose: key.purpose.clone(),
                        scope: key.scope.clone(),
                        channel: key.channel.clone(),
                        state: String::new(),
                        granted_at: None,
                        withdrawn_at: None,
                        subject_value: subject_value(subject),
                    },
                });
                let last = self.records.len() - 1;
                &mut self.records[last]
            }
        };
        record.consent.state = state.to_owned();
        if state == GRANTED {
            record.consent.granted_at = Some(now);
            record.consent.withdrawn_at = None;
        } else {
            record.consent.withdrawn_at = Some(now);
        }
        record.consent.clone()
    }

    fn current(&self, subject: &ConsentSubject, key: &ConsentKey) -> Option<Consent> {
        self.records
            .iter()
            .find(|record| {
                record.subject == *subject
                    && record.consent.purpose == key.purpose
                    && record.consent.scope == key.scope
                    && record.consent.channel == key.channel
            })
            .map(|record| record.consent.clone())
    }

    fn granted(&self, subject: &ConsentSubject, key: &ConsentKey) -> bool {
        let covering: Vec<&Consent> = self
            .records
            .iter()
            .filter(|record| record.subject == *subject && covers(&record.consent, key))
            .map(|record| &record.consent)
            .collect();
        covering.iter().any(|consent| consent.state == GRANTED)
            && !covering.iter().any(|consent| consent.state == WITHDRAWN)
    }

    fn check(&self) -> Result<(), Error> {
        if self.failing {
            Err(Error::internal("FakeConsentLedger is failing (set by the test)"))
        } else {
            Ok(())
        }
    }
}

impl FakeConsentLedger {
    pub fn new() -> Self {
        FakeConsentLedger::default()
    }

    /// Ghi đồng ý ngay (dựng dữ liệu cho test, không qua module); trả dòng sau khi ghi.
    pub fn allow(&self, subject: ConsentSubject, key: ConsentKey) -> Consent {
        lock(&self.state).set(&subject, &key, GRANTED)
    }

    /// Ghi rút lại ngay ở khoá `key` (rút ở khoá rộng hơn chặn mọi khoá con); trả dòng sau khi ghi.
    pub fn deny(&self, subject: ConsentSubject, key: ConsentKey) -> Consent {
        lock(&self.state).set(&subject, &key, WITHDRAWN)
    }

    /// Bỏ mọi dòng của người `subject` (như chưa từng đồng ý gì).
    pub fn forget(&self, subject: &ConsentSubject) {
        lock(&self.state).records.retain(|record| record.subject != *subject);
    }

    /// Trạng thái (`granted` / `withdrawn`) ở đúng khoá `key`, nếu có dòng.
    pub fn state_of(&self, subject: &ConsentSubject, key: &ConsentKey) -> Option<String> {
        lock(&self.state).current(subject, key).map(|consent| consent.state)
    }

    /// Mọi dòng của sổ, theo thứ tự tạo.
    pub fn records(&self) -> Vec<ConsentRecord> {
        lock(&self.state).records.clone()
    }

    /// Các `source_ref` của lời gọi `grant` đã ghi, theo thứ tự.
    pub fn source_refs(&self) -> Vec<String> {
        lock(&self.state).source_refs.clone()
    }

    /// Bật / tắt chế độ "sổ hỏng": mọi lời gọi trả `internal_error`.
    pub fn set_failing(&self, failing: bool) {
        lock(&self.state).failing = failing;
    }
}

#[async_trait::async_trait]
impl ConsentLedger for FakeConsentLedger {
    async fn grant(
        &self,
        _: TenantId,
        subject: ConsentSubject,
        key: ConsentKey,
        evidence: ConsentEvidence,
        source_ref: &str,
    ) -> Result<Consent, Error> {
        let mut state = lock(&self.state);
        state.check()?;
        if evidence.text.is_empty() || evidence.text_version.is_empty() {
            return Err(Error::internal("FakeConsentLedger: a consent needs its text and version"));
        }
        if state.source_refs.iter().any(|known| known == source_ref)
            && let Some(current) = state.current(&subject, &key)
        {
            return Ok(current);
        }
        state.source_refs.push(source_ref.to_owned());
        Ok(state.set(&subject, &key, GRANTED))
    }

    async fn withdraw(
        &self,
        _: TenantId,
        subject: ConsentSubject,
        key: ConsentKey,
        _: &str,
        _: &str,
    ) -> Result<Consent, Error> {
        let mut state = lock(&self.state);
        state.check()?;
        Ok(state.set(&subject, &key, WITHDRAWN))
    }

    async fn is_granted(
        &self,
        _: TenantId,
        subject: &ConsentSubject,
        key: &ConsentKey,
    ) -> Result<bool, Error> {
        let state = lock(&self.state);
        state.check()?;
        Ok(state.granted(subject, key))
    }

    async fn granted_many(
        &self,
        _: TenantId,
        subjects: &[ConsentSubject],
        key: &ConsentKey,
    ) -> Result<Vec<bool>, Error> {
        let state = lock(&self.state);
        state.check()?;
        Ok(subjects.iter().map(|subject| state.granted(subject, key)).collect())
    }

    async fn get(&self, _: TenantId, consent_id: Uuid) -> Result<Option<Consent>, Error> {
        let state = lock(&self.state);
        state.check()?;
        Ok(state
            .records
            .iter()
            .find(|record| record.consent.id == consent_id)
            .map(|record| record.consent.clone()))
    }
}

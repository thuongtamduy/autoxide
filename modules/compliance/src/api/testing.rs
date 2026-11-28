//! A shared fake consent ledger for other modules' tests (feature `testing`, declared only in
//! `[dev-dependencies]`; convention D-20261122-module-test-fakes).
//!
//! Like the real `compliance` in the ways other modules rely on:
//! - one row per (person, key), state `granted` / `withdrawn`; calling `grant` again with the same `source_ref`
//!   records nothing more and returns the current state;
//! - `is_granted`: whether a consent exists at exactly this key or a broader key (an empty `scope` /
//!   `channel` covers the children), and there is no withdrawal at this key or a broader key;
//! - a consent must have wording and a wording version.
//!
//! Does not distinguish tenants (`TenantId`): one ledger per test.
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

/// One row of the fake ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsentRecord {
    pub subject: ConsentSubject,
    pub consent: Consent,
}

#[derive(Default)]
struct State {
    records: Vec<ConsentRecord>,
    /// `source_ref` values seen, in order (a repeat call adds none).
    source_refs: Vec<String>,
    /// A "broken" ledger: every call returns an error.
    failing: bool,
}

/// A fake consent ledger that keeps everything in memory. Publish it into the app with
/// `TestApp::builder(…).provide::<dyn ConsentLedger>(consents.clone())`.
#[derive(Default)]
pub struct FakeConsentLedger {
    state: Mutex<State>,
}

// Khoá `Mutex` mà không `unwrap`: test khác panic giữa chừng thì vẫn đọc tiếp dữ liệu bên trong.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// `broad` covers `key`: same purpose, and the scope and channel are empty or equal.
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

    /// Records a consent immediately (builds test data, bypassing the module); returns the row after writing.
    pub fn allow(&self, subject: ConsentSubject, key: ConsentKey) -> Consent {
        lock(&self.state).set(&subject, &key, GRANTED)
    }

    /// Records a withdrawal immediately at key `key` (a withdrawal at a broader key blocks every child key); returns the row after writing.
    pub fn deny(&self, subject: ConsentSubject, key: ConsentKey) -> Consent {
        lock(&self.state).set(&subject, &key, WITHDRAWN)
    }

    /// Removes every row of person `subject` (as if they never consented to anything).
    pub fn forget(&self, subject: &ConsentSubject) {
        lock(&self.state).records.retain(|record| record.subject != *subject);
    }

    /// State (`granted` / `withdrawn`) at exactly key `key`, if there is a row.
    pub fn state_of(&self, subject: &ConsentSubject, key: &ConsentKey) -> Option<String> {
        lock(&self.state).current(subject, key).map(|consent| consent.state)
    }

    /// Every row of the ledger, in creation order.
    pub fn records(&self) -> Vec<ConsentRecord> {
        lock(&self.state).records.clone()
    }

    /// The `source_ref` values of `grant` calls recorded, in order.
    pub fn source_refs(&self) -> Vec<String> {
        lock(&self.state).source_refs.clone()
    }

    /// Turns the "broken ledger" mode on / off: every call returns `internal_error`.
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

/// The error the real gate returns when `activity` needs a verified identity the subject does not have
/// (`compliance.kyc_required`, 403), for a fake `KycGate` in another module's tests.
pub fn kyc_required(activity: super::Activity) -> Error {
    Error::new(
        autoxide_kernel::error::ErrorKind::Forbidden,
        "compliance.kyc_required",
        "A verified identity is required to {activity}; the verification profile is `{profile_status}`.",
    )
    .with_param("activity", activity.as_str())
    .with_param("profile_status", "none")
}

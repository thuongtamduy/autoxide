//! [`FakeDns`]: a fake TXT resolver for custom-domain tests — never queries real DNS. Every
//! [`TestApp`](crate::TestApp) gets one ready to use (`app.dns()`); tests set records with
//! [`FakeDns::set_txt`].

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{CHALLENGE_PREFIX, CHALLENGE_VALUE_PREFIX, DnsTxtLookup};

/// TXT records kept in memory. Clones share the records.
#[derive(Clone, Default)]
pub struct FakeDns {
    records: Arc<Mutex<HashMap<String, Vec<String>>>>,
    failing: Arc<Mutex<bool>>,
    held: Arc<Mutex<Option<DnsHold>>>,
}

/// Lookups held by [`FakeDns::hold`]: each lookup waits until [`release`](Self::release). Lets a test act at the
/// exact moment a domain check is between its DNS lookup and its write (e.g. another tenant verifies the
/// same domain first). Clones share the hold.
#[derive(Clone, Debug)]
pub struct DnsHold {
    started: Arc<tokio::sync::Notify>,
    gate: Arc<tokio::sync::Semaphore>,
}

impl DnsHold {
    /// Waits until a lookup has started (and is held). Returns at once if one already started.
    pub async fn started(&self) {
        self.started.notified().await;
    }

    /// Lets every held and later lookup go on.
    pub fn release(&self) {
        self.gate.add_permits(tokio::sync::Semaphore::MAX_PERMITS / 2);
    }
}

impl std::fmt::Debug for FakeDns {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeDns").finish_non_exhaustive()
    }
}

impl FakeDns {
    /// Sets (replaces) all TXT values of `name`.
    pub fn set_txt(&self, name: &str, values: &[&str]) {
        let values = values.iter().map(|value| (*value).to_owned()).collect();
        self.lock().insert(key(name), values);
    }

    /// Sets the verification record of `domain`: `_autoxide-challenge.<domain>` =
    /// `autoxide-verify=<token>`.
    pub fn set_challenge(&self, domain: &str, token: &str) {
        let value = format!("{CHALLENGE_VALUE_PREFIX}{token}");
        self.set_txt(&format!("{CHALLENGE_PREFIX}.{domain}"), &[&value]);
    }

    /// Removes all records of `name`.
    pub fn remove(&self, name: &str) {
        self.lock().remove(&key(name));
    }

    /// Holds every lookup from now on until the returned [`DnsHold`] is released.
    pub fn hold(&self) -> DnsHold {
        let hold = DnsHold {
            started: Arc::new(tokio::sync::Notify::new()),
            gate: Arc::new(tokio::sync::Semaphore::new(0)),
        };
        *self.held.lock().unwrap_or_else(PoisonError::into_inner) = Some(hold.clone());
        hold
    }

    /// `true`: every lookup returns an error (as when the DNS server does not answer).
    pub fn fail(&self, failing: bool) {
        *self.failing.lock().unwrap_or_else(PoisonError::into_inner) = failing;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Vec<String>>> {
        self.records.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn key(name: &str) -> String {
    name.trim().trim_end_matches('.').to_ascii_lowercase()
}

#[async_trait::async_trait]
impl DnsTxtLookup for FakeDns {
    async fn txt(&self, name: &str) -> Result<Vec<String>, Error> {
        let held = self.held.lock().unwrap_or_else(PoisonError::into_inner).clone();
        if let Some(hold) = held {
            // `notify_one` giữ một "vé" khi chưa ai chờ: `started()` gọi sau vẫn thấy.
            hold.started.notify_one();
            // Semaphore chỉ đóng khi bị `close()` — ở đây không bao giờ: lỗi coi như đã thả.
            let _ = hold.gate.acquire().await;
        }
        if *self.failing.lock().unwrap_or_else(PoisonError::into_inner) {
            return Err(autoxide_kernel::tenancy::errors::tenant_domain_dns_failed(
                "the fake DNS server is down",
            ));
        }
        Ok(self.lock().get(&key(name)).cloned().unwrap_or_default())
    }
}

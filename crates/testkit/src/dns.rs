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
        if *self.failing.lock().unwrap_or_else(PoisonError::into_inner) {
            return Err(autoxide_kernel::tenancy::errors::tenant_domain_dns_failed(
                "the fake DNS server is down",
            ));
        }
        Ok(self.lock().get(&key(name)).cloned().unwrap_or_default())
    }
}

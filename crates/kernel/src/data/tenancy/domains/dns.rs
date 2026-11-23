//! The real TXT resolver: `hickory-resolver` with the operating system's DNS configuration (`/etc/resolv.conf`
//! on Unix, the registry on Windows). Built lazily on the first lookup: a machine without DNS configuration
//! still starts, and only domain verification reports an error.

use std::sync::OnceLock;
use std::time::Duration;

use hickory_resolver::TokioResolver;
use hickory_resolver::proto::rr::RData;

use super::{DnsTxtLookup, tenant_domain_dns_failed};
use crate::error::Error;

/// A DNS answer (record present or absent) stays in the resolver cache at most this long: after an
/// administrator just edits a TXT record, the verification a minute later sees the new one.
const CACHE_TTL: Duration = Duration::from_secs(60);

#[derive(Default)]
pub(super) struct SystemDns {
    resolver: OnceLock<TokioResolver>,
}

impl SystemDns {
    fn resolver(&self) -> Result<&TokioResolver, Error> {
        if let Some(resolver) = self.resolver.get() {
            return Ok(resolver);
        }
        let mut builder = TokioResolver::builder_tokio().map_err(|error| {
            tenant_domain_dns_failed(&format!("cannot read the system DNS configuration: {error}"))
        })?;
        let options = builder.options_mut();
        options.positive_max_ttl = Some(CACHE_TTL);
        options.negative_max_ttl = Some(CACHE_TTL);
        let resolver = builder.build().map_err(|error| {
            tenant_domain_dns_failed(&format!("cannot start the DNS resolver: {error}"))
        })?;
        // Hai request dựng cùng lúc: giữ bản đầu tiên, bản kia bỏ đi.
        Ok(self.resolver.get_or_init(|| resolver))
    }
}

#[async_trait::async_trait]
impl DnsTxtLookup for SystemDns {
    async fn txt(&self, name: &str) -> Result<Vec<String>, Error> {
        let resolver = self.resolver()?;
        // Dấu chấm cuối: tên đầy đủ, không ghép thêm `search` của resolv.conf.
        let fqdn = format!("{}.", name.trim_end_matches('.'));
        match resolver.txt_lookup(fqdn.as_str()).await {
            Ok(lookup) => Ok(lookup
                .answers()
                .iter()
                .filter_map(|record| match &record.data {
                    // Một bản ghi TXT có thể gồm nhiều chuỗi ≤ 255 byte: nối lại thành một giá trị.
                    RData::TXT(txt) => {
                        Some(String::from_utf8_lossy(&txt.txt_data.concat()).into_owned())
                    }
                    _ => None,
                })
                .collect()),
            Err(error) if error.is_no_records_found() => Ok(Vec::new()),
            Err(error) => Err(tenant_domain_dns_failed(&error.to_string())),
        }
    }
}

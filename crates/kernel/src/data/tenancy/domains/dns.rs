//! Bộ tra TXT thật: `hickory-resolver` với cấu hình DNS của hệ điều hành (`/etc/resolv.conf` trên
//! Unix, registry trên Windows). Dựng lười ở lần tra đầu: máy không có cấu hình DNS vẫn khởi động
//! được, chỉ việc xác minh tên miền báo lỗi.

use std::sync::OnceLock;
use std::time::Duration;

use hickory_resolver::TokioResolver;
use hickory_resolver::proto::rr::RData;

use super::{DnsTxtLookup, tenant_domain_dns_failed};
use crate::error::Error;

/// Câu trả lời DNS (có hay không có bản ghi) giữ trong cache của resolver tối đa chừng này: người
/// quản trị vừa sửa bản ghi TXT thì lần xác minh sau đó một phút thấy bản mới.
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

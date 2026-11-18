//! Loại sự kiện phân tích truy cập và phễu do module khai (bước 57, D-220): module nói "website /
//! máy chủ của tôi sinh sự kiện gì" bằng [`AnalyticsEvent`] (`Module::analytics_events`) và "phễu nào
//! đáng đo" bằng [`Funnel`] (`Module::funnels`). Module `analytics` đọc [`AnalyticsCatalog`] lúc khởi
//! động: nhận sự kiện từ website chỉ khi loại đã khai (loại lạ bị bỏ, không lỗi), và nghe event của
//! module cho sự kiện phía máy chủ (không tin số đơn do trình duyệt gửi). Kernel không biết nghĩa
//! nghiệp vụ của sự kiện: chỉ giữ hợp đồng và kiểm khai báo lúc khởi động.
//!
//! ```
//! use autoxide_kernel::analytics::{AnalyticsEvent, Funnel, Prop, PropKind};
//!
//! const VIEWED: AnalyticsEvent = AnalyticsEvent::browser(
//!     "catalog.product_viewed",
//!     "A shopper opened the page of a product.",
//! )
//! .object_kind("catalog.product")
//! .props(&[Prop::new("variant", PropKind::Text)]);
//!
//! const CHECKOUT: Funnel = Funnel::new(
//!     "orders.checkout",
//!     "From product page to paid order.",
//!     &["catalog.product_viewed", "orders.cart_added", "orders.purchased"],
//! );
//! assert_eq!(VIEWED.key, "catalog.product_viewed");
//! assert_eq!(CHECKOUT.steps.len(), 3);
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use crate::error::Error;
use crate::event::{Delivered, EventCatalog};
use crate::tenancy::OrgId;

/// Số thuộc tính (`props`) tối đa của một loại sự kiện.
pub const PROPS_MAX: usize = 10;

/// Số bước tối đa của một phễu.
pub const FUNNEL_STEPS_MAX: usize = 10;

/// Kiểu giá trị của một thuộc tính.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropKind {
    /// Chuỗi, tối đa 100 ký tự.
    Text,
    /// Số (nguyên hay thực).
    Number,
    Boolean,
}

impl PropKind {
    /// Tên ổn định trong API: `text`, `number`, `boolean`.
    pub const fn as_str(self) -> &'static str {
        match self {
            PropKind::Text => "text",
            PropKind::Number => "number",
            PropKind::Boolean => "boolean",
        }
    }

    /// `value` có đúng kiểu không (chuỗi quá 100 ký tự là sai).
    pub fn accepts(self, value: &Value) -> bool {
        match (self, value) {
            (PropKind::Text, Value::String(text)) => text.chars().count() <= PROP_TEXT_MAX,
            (PropKind::Number, Value::Number(_)) | (PropKind::Boolean, Value::Bool(_)) => true,
            _ => false,
        }
    }
}

/// Độ dài tối đa của thuộc tính kiểu chữ.
pub const PROP_TEXT_MAX: usize = 100;

/// Một thuộc tính được phép của loại sự kiện.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prop {
    pub name: &'static str,
    pub kind: PropKind,
}

impl Prop {
    pub const fn new(name: &'static str, kind: PropKind) -> Self {
        Prop { name, kind }
    }
}

/// Một sự kiện phía máy chủ rút từ event của module: không có phiên của trình duyệt.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ServerHit {
    /// Website (nút) của sự kiện; `None`: nút gốc của khách.
    pub org_id: Option<OrgId>,
    pub occurred_at: DateTime<Utc>,
    /// Đối tượng, dạng `<loại>:<id>` (vd `catalog.product:0190…`).
    pub object: Option<String>,
    /// Người làm. Module `analytics` chỉ giữ khi người đó đồng ý theo dõi theo người.
    pub user_id: Option<Uuid>,
}

impl ServerHit {
    pub fn new(occurred_at: DateTime<Utc>) -> Self {
        ServerHit { org_id: None, occurred_at, object: None, user_id: None }
    }

    #[must_use]
    pub fn org(mut self, org: Option<OrgId>) -> Self {
        self.org_id = org;
        self
    }

    #[must_use]
    pub fn object(mut self, object: impl Into<String>) -> Self {
        self.object = Some(object.into());
        self
    }

    #[must_use]
    pub fn user(mut self, user_id: Option<Uuid>) -> Self {
        self.user_id = user_id;
        self
    }
}

/// Hàm rút sự kiện từ một event của module: thuần, không I/O. `None`: event này không tính.
pub type ExtractHit = fn(&Delivered) -> Result<Option<ServerHit>, Error>;

/// Nguồn của một loại sự kiện.
#[derive(Clone, Copy)]
pub enum Source {
    /// Website gửi qua endpoint công khai.
    Browser,
    /// Rút từ event `event` của chính module; website không gửi được loại này.
    Server { event: &'static str, extract: ExtractHit },
}

impl std::fmt::Debug for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Source::Browser => f.write_str("Browser"),
            Source::Server { event, .. } => f.debug_struct("Server").field("event", event).finish(),
        }
    }
}

/// Một loại sự kiện module khai.
#[derive(Debug, Clone, Copy)]
pub struct AnalyticsEvent {
    /// `<module>.<sự kiện>`, vd `catalog.product_viewed`.
    pub key: &'static str,
    /// Câu tiếng Anh cho người gọi API.
    pub description: &'static str,
    /// Loại đối tượng sự kiện nói tới (vd `catalog.product`): `object` của sự kiện phải là
    /// `<loại này>:<id>`. `None`: sự kiện không có đối tượng.
    pub object_kind: Option<&'static str>,
    /// Thuộc tính được phép; thuộc tính lạ hay sai kiểu làm sự kiện bị bỏ.
    pub props: &'static [Prop],
    pub source: Source,
}

impl AnalyticsEvent {
    /// Loại sự kiện website gửi.
    pub const fn browser(key: &'static str, description: &'static str) -> Self {
        AnalyticsEvent { key, description, object_kind: None, props: &[], source: Source::Browser }
    }

    /// Loại sự kiện rút từ event `event` của chính module (vd `orders.order_confirmed` →
    /// `orders.purchased`).
    pub const fn server(
        key: &'static str,
        description: &'static str,
        event: &'static str,
        extract: ExtractHit,
    ) -> Self {
        AnalyticsEvent {
            key,
            description,
            object_kind: None,
            props: &[],
            source: Source::Server { event, extract },
        }
    }

    #[must_use]
    pub const fn object_kind(mut self, kind: &'static str) -> Self {
        self.object_kind = Some(kind);
        self
    }

    #[must_use]
    pub const fn props(mut self, props: &'static [Prop]) -> Self {
        self.props = props;
        self
    }

    /// Website gửi được loại này.
    pub fn from_browser(&self) -> bool {
        matches!(self.source, Source::Browser)
    }
}

/// Một phễu module khai: các bước là loại sự kiện (của bất kỳ module nào), theo thứ tự.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Funnel {
    /// `<module>.<phễu>`, vd `orders.checkout`.
    pub key: &'static str,
    pub description: &'static str,
    pub steps: &'static [&'static str],
}

impl Funnel {
    pub const fn new(
        key: &'static str,
        description: &'static str,
        steps: &'static [&'static str],
    ) -> Self {
        Funnel { key, description, steps }
    }
}

#[derive(Debug, Default)]
struct Inner {
    events: Vec<(&'static str, AnalyticsEvent)>,
    funnels: Vec<(&'static str, Funnel)>,
}

/// Mọi loại sự kiện và phễu của mọi module đã đăng ký, đã kiểm. Clone rẻ.
#[derive(Debug, Clone, Default)]
pub struct AnalyticsCatalog(Arc<Inner>);

/// Khai báo của một module: `(module, loại sự kiện, phễu)`.
pub type Declared = (&'static str, Vec<AnalyticsEvent>, Vec<Funnel>);

impl AnalyticsCatalog {
    /// Kiểm và gom khai báo của các module. `events`: mọi event đã khai — sự kiện phía máy chủ
    /// chỉ nghe event của chính module. Trả mọi vấn đề cùng lúc.
    pub fn new(modules: Vec<Declared>, events: &EventCatalog) -> Result<Self, Vec<String>> {
        let mut problems = Vec::new();
        let mut inner = Inner::default();
        let mut keys = BTreeSet::new();
        for (module, declared, _) in &modules {
            for event in declared {
                if !keys.insert(event.key) {
                    problems.push(format!("analytics event `{}` is declared twice", event.key));
                }
                problems.extend(event_problems(module, event, events));
                inner.events.push((module, *event));
            }
        }
        let mut funnel_keys = BTreeSet::new();
        for (module, _, funnels) in modules {
            for funnel in funnels {
                if !funnel_keys.insert(funnel.key) {
                    problems.push(format!("funnel `{}` is declared twice", funnel.key));
                }
                if !owned_by(module, funnel.key) {
                    problems.push(format!(
                        "funnel `{}` of module `{module}` must be named `{module}.<funnel>`, in \
                         lowercase letters, digits and `_`",
                        funnel.key
                    ));
                }
                if funnel.description.trim().is_empty() {
                    problems.push(format!("funnel `{}` has no description", funnel.key));
                }
                if funnel.steps.len() < 2 || funnel.steps.len() > FUNNEL_STEPS_MAX {
                    problems.push(format!(
                        "funnel `{}` must have 2 to {FUNNEL_STEPS_MAX} steps",
                        funnel.key
                    ));
                }
                for step in funnel.steps {
                    if !keys.contains(step) {
                        problems.push(format!(
                            "step `{step}` of funnel `{}` is not a declared analytics event",
                            funnel.key
                        ));
                    }
                }
                inner.funnels.push((module, funnel));
            }
        }
        if problems.is_empty() { Ok(AnalyticsCatalog(Arc::new(inner))) } else { Err(problems) }
    }

    /// Mọi loại sự kiện, theo thứ tự đăng ký module.
    pub fn events(&self) -> impl Iterator<Item = &AnalyticsEvent> {
        self.0.events.iter().map(|(_, event)| event)
    }

    /// Loại sự kiện `key`, nếu có.
    pub fn event(&self, key: &str) -> Option<&AnalyticsEvent> {
        self.events().find(|event| event.key == key)
    }

    /// Mọi phễu, theo thứ tự đăng ký module.
    pub fn funnels(&self) -> impl Iterator<Item = &Funnel> {
        self.0.funnels.iter().map(|(_, funnel)| funnel)
    }

    /// Phễu `key`, nếu có.
    pub fn funnel(&self, key: &str) -> Option<&Funnel> {
        self.funnels().find(|funnel| funnel.key == key)
    }
}

fn owned_by(module: &str, name: &str) -> bool {
    name.split_once('.')
        .is_some_and(|(prefix, rest)| prefix == module && crate::module::is_identifier(rest))
}

fn event_problems(module: &str, event: &AnalyticsEvent, events: &EventCatalog) -> Vec<String> {
    let mut problems = Vec::new();
    if !owned_by(module, event.key) {
        problems.push(format!(
            "analytics event `{}` of module `{module}` must be named `{module}.<event>`, in \
             lowercase letters, digits and `_`",
            event.key
        ));
    }
    if event.description.trim().is_empty() {
        problems.push(format!("analytics event `{}` has no description", event.key));
    }
    if let Some(kind) = event.object_kind
        && !kind.split_once('.').is_some_and(|(a, b)| {
            crate::module::is_identifier(a) && crate::module::is_identifier(b)
        })
    {
        problems.push(format!(
            "object kind `{kind}` of analytics event `{}` must look like `<module>.<kind>`",
            event.key
        ));
    }
    if event.props.len() > PROPS_MAX {
        problems
            .push(format!("analytics event `{}` declares more than {PROPS_MAX} props", event.key));
    }
    let mut names = BTreeMap::new();
    for prop in event.props {
        if !crate::module::is_identifier(prop.name) {
            problems.push(format!(
                "prop `{}` of analytics event `{}` must be lowercase letters, digits and `_`",
                prop.name, event.key
            ));
        }
        if names.insert(prop.name, ()).is_some() {
            problems.push(format!(
                "prop `{}` of analytics event `{}` is declared twice",
                prop.name, event.key
            ));
        }
    }
    if let Source::Server { event: name, .. } = event.source {
        if !owned_by(module, name) {
            problems.push(format!(
                "analytics event `{}` of module `{module}` reads event `{name}`: a module only \
                 reads its own events (`{module}.<event>`)",
                event.key
            ));
        } else if !events.contains(name) {
            problems.push(format!(
                "analytics event `{}` of module `{module}` reads event `{name}`, which the module \
                 does not declare in Module::events",
                event.key
            ));
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::EventInfo;

    fn none(_: &Delivered) -> Result<Option<ServerHit>, Error> {
        Ok(None)
    }

    fn catalog(declared: Vec<Declared>) -> Result<AnalyticsCatalog, Vec<String>> {
        let events = EventCatalog::new(vec![(
            "orders",
            vec![EventInfo::new("orders.order_confirmed", "An order was confirmed.")],
        )])
        .unwrap();
        AnalyticsCatalog::new(declared, &events)
    }

    const VARIANT: &[Prop] = &[Prop::new("variant", PropKind::Text)];

    #[test]
    fn khai_dung_thi_tra_duoc() {
        let view = AnalyticsEvent::browser("catalog.product_viewed", "Viewed.")
            .object_kind("catalog.product")
            .props(VARIANT);
        let bought =
            AnalyticsEvent::server("orders.purchased", "Bought.", "orders.order_confirmed", none);
        let funnel = Funnel::new(
            "orders.checkout",
            "Checkout.",
            &["catalog.product_viewed", "orders.purchased"],
        );
        let catalog =
            catalog(vec![("catalog", vec![view], vec![]), ("orders", vec![bought], vec![funnel])])
                .unwrap();
        assert!(catalog.event("catalog.product_viewed").unwrap().from_browser());
        assert!(!catalog.event("orders.purchased").unwrap().from_browser());
        assert_eq!(catalog.funnel("orders.checkout").unwrap().steps.len(), 2);
        assert!(catalog.event("orders.unknown").is_none());
    }

    #[test]
    fn khai_sai_bao_moi_van_de() {
        let foreign = AnalyticsEvent::server("orders.purchased", "Bought.", "catalog.thing", none);
        let wrong_name = AnalyticsEvent::browser("other.viewed", " ");
        let funnel = Funnel::new("orders.checkout", "Checkout.", &["orders.purchased"]);
        let problems =
            catalog(vec![("orders", vec![foreign, wrong_name], vec![funnel])]).unwrap_err();
        assert!(problems.iter().any(|p| p.contains("only reads its own events")), "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("must be named `orders.<event>`")));
        assert!(problems.iter().any(|p| p.contains("has no description")));
        assert!(problems.iter().any(|p| p.contains("2 to 10 steps")));
    }

    #[test]
    fn kieu_thuoc_tinh() {
        assert!(PropKind::Text.accepts(&Value::from("a")));
        assert!(!PropKind::Text.accepts(&Value::from("a".repeat(101))));
        assert!(PropKind::Number.accepts(&Value::from(1.5)));
        assert!(!PropKind::Boolean.accepts(&Value::from(1)));
    }
}

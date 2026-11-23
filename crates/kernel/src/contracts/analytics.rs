//! Analytics event types and funnels declared by modules (step 57, D-220): a module says "which events my
//! website / server produces" with [`AnalyticsEvent`] (`Module::analytics_events`) and "which funnels are
//! worth measuring" with [`Funnel`] (`Module::funnels`). The `analytics` module reads the
//! [`AnalyticsCatalog`] at startup: it accepts website events only for declared types (unknown types are
//! dropped, not an error), and listens to the module's events for server-side analytics events (it does not
//! trust order counts sent by the browser). The kernel does not know what an event means for the business: it
//! only holds the contract and validates the declarations at startup.
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

use crate::time::Timestamp;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use uuid::Uuid;

use crate::error::Error;
use crate::event::{Delivered, EventCatalog};
use crate::tenancy::OrgId;

/// Maximum number of properties (`props`) of one event type.
pub const PROPS_MAX: usize = 10;

/// Maximum number of steps of one funnel.
pub const FUNNEL_STEPS_MAX: usize = 10;

/// Value type of a property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropKind {
    /// String, at most 100 characters.
    Text,
    /// Number (integer or floating point).
    Number,
    Boolean,
}

impl PropKind {
    /// Stable name in the API: `text`, `number`, `boolean`.
    pub const fn as_str(self) -> &'static str {
        match self {
            PropKind::Text => "text",
            PropKind::Number => "number",
            PropKind::Boolean => "boolean",
        }
    }

    /// Whether `value` has the right type (a string longer than 100 characters is wrong).
    pub fn accepts(self, value: &Value) -> bool {
        match (self, value) {
            (PropKind::Text, Value::String(text)) => text.chars().count() <= PROP_TEXT_MAX,
            (PropKind::Number, Value::Number(_)) | (PropKind::Boolean, Value::Bool(_)) => true,
            _ => false,
        }
    }
}

/// Maximum length of a text property.
pub const PROP_TEXT_MAX: usize = 100;

/// A property allowed on an event type.
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

/// A server-side event derived from a module event: it has no browser session.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ServerHit {
    /// Website (node) of the event; `None`: the tenant's root node.
    pub org_id: Option<OrgId>,
    pub occurred_at: Timestamp,
    /// Object, in the form `<kind>:<id>` (e.g. `catalog.product:0190…`).
    pub object: Option<String>,
    /// The actor. The `analytics` module keeps it only if the person consented to per-person tracking.
    pub user_id: Option<Uuid>,
}

impl ServerHit {
    pub fn new(occurred_at: Timestamp) -> Self {
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

/// Function that derives an analytics event from a module event: pure, no I/O. `None`: this event does not count.
pub type ExtractHit = fn(&Delivered) -> Result<Option<ServerHit>, Error>;

/// Source of an event type.
#[derive(Clone, Copy)]
pub enum Source {
    /// The website sends it through the public endpoint.
    Browser,
    /// Derived from the module's own event `event`; the website cannot send this type.
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

/// An event type declared by a module.
#[derive(Debug, Clone, Copy)]
pub struct AnalyticsEvent {
    /// `<module>.<event>`, e.g. `catalog.product_viewed`.
    pub key: &'static str,
    /// English sentence for API callers.
    pub description: &'static str,
    /// Kind of object the event refers to (e.g. `catalog.product`): the event's `object` must be
    /// `<this kind>:<id>`. `None`: the event has no object.
    pub object_kind: Option<&'static str>,
    /// Allowed properties; an unknown property or one of the wrong type causes the event to be dropped.
    pub props: &'static [Prop],
    pub source: Source,
}

impl AnalyticsEvent {
    /// An event type the website sends.
    pub const fn browser(key: &'static str, description: &'static str) -> Self {
        AnalyticsEvent { key, description, object_kind: None, props: &[], source: Source::Browser }
    }

    /// An event type derived from the module's own event `event` (e.g. `orders.order_confirmed` →
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

    /// The website can send this type.
    pub fn from_browser(&self) -> bool {
        matches!(self.source, Source::Browser)
    }
}

/// A funnel declared by a module: the steps are event types (of any module), in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Funnel {
    /// `<module>.<funnel>`, e.g. `orders.checkout`.
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

/// All event types and funnels of all registered modules, validated. Cheap to clone.
#[derive(Debug, Clone, Default)]
pub struct AnalyticsCatalog(Arc<Inner>);

/// Declarations of one module: `(module, event types, funnels)`.
pub type Declared = (&'static str, Vec<AnalyticsEvent>, Vec<Funnel>);

impl AnalyticsCatalog {
    /// Validates and collects the modules' declarations. `events`: every declared event — server-side events
    /// only listen to the module's own events. Returns all problems at once.
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

    /// All event types, in module registration order.
    pub fn events(&self) -> impl Iterator<Item = &AnalyticsEvent> {
        self.0.events.iter().map(|(_, event)| event)
    }

    /// The event type `key`, if any.
    pub fn event(&self, key: &str) -> Option<&AnalyticsEvent> {
        self.events().find(|event| event.key == key)
    }

    /// All funnels, in module registration order.
    pub fn funnels(&self) -> impl Iterator<Item = &Funnel> {
        self.0.funnels.iter().map(|(_, funnel)| funnel)
    }

    /// The funnel `key`, if any.
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

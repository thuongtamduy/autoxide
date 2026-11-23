//! Facts for consolidated reports (step 38, D-20261118-step38-report-facts): a module declares "which of its
//! events produce which facts" with [`FactSource`] (`Module::facts`); the `reporting` module listens to those
//! events, extracts [`Fact`]s and pours them into the report store of the tenant tree. The kernel does not
//! know the business meaning of the facts: it only holds the contract and validates declarations at startup.
//!
//! ```
//! use autoxide_kernel::report::{Fact, FactKind, FactSource, Measure, money_field, org_field};
//!
//! const ISSUED_TOTAL: FactKind = FactKind::new(
//!     "invoices.issued_total",
//!     "Total of the documents issued.",
//!     Measure::Amount,
//!     &["kind"],
//! );
//!
//! let source = FactSource::new("invoices.document_issued", vec![ISSUED_TOTAL], |event| {
//!     let mut fact = Fact::new(ISSUED_TOTAL.kind, event.occurred_at).org(org_field(&event.payload));
//!     if let Some(total) = money_field(&event.payload, "total")? {
//!         fact = fact.amount(total);
//!     }
//!     Ok(vec![fact])
//! });
//! assert_eq!(source.event(), "invoices.document_issued");
//! ```

use crate::time::Timestamp;
use serde_json::Value;
use std::collections::BTreeMap;
use uuid::Uuid;

use crate::error::Error;
use crate::event::{Delivered, EventCatalog};
use crate::money::Money;
use crate::tenancy::OrgId;

/// Unit of a kind of fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Measure {
    /// An amount (the transaction's original money; reports sum per currency).
    Amount,
    /// A count / whole quantity.
    Quantity,
}

impl Measure {
    /// Stable name in the API: `amount`, `quantity`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Measure::Amount => "amount",
            Measure::Quantity => "quantity",
        }
    }
}

/// A kind of fact a module declares (`GET /api/v1/reporting/kinds`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FactKind {
    /// `<module>.<metric>`, vd `invoices.issued_total`.
    pub kind: &'static str,
    /// English sentence for API callers.
    pub description: &'static str,
    pub measure: Measure,
    /// Allowed dimensions — a small fixed set; values are codes (not people's names).
    pub dimensions: &'static [&'static str],
}

impl FactKind {
    pub const fn new(
        kind: &'static str,
        description: &'static str,
        measure: Measure,
        dimensions: &'static [&'static str],
    ) -> Self {
        FactKind { kind, description, measure, dimensions }
    }
}

/// A report fact, extracted from an event. Contains NO personal data (name, email, phone number,
/// address, `user_id`): the consolidated report store lives in the root tenant's database and is not erased
/// per person.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Fact {
    /// A kind declared in the source's own [`FactSource::kinds`].
    pub kind: &'static str,
    /// Owning node in the source tenant; `None`: the tenant's root node.
    pub org_id: Option<OrgId>,
    pub occurred_at: Timestamp,
    /// The amount — present when the kind is [`Measure::Amount`].
    pub amount: Option<Money>,
    /// The count — present when the kind is [`Measure::Quantity`].
    pub quantity: Option<i64>,
    /// Value per dimension; the key must be in [`FactKind::dimensions`].
    pub dimensions: BTreeMap<&'static str, String>,
}

impl Fact {
    /// A fact of kind `kind` that occurred at `occurred_at`, with no amount, count or dimension yet.
    pub fn new(kind: &'static str, occurred_at: Timestamp) -> Self {
        Fact {
            kind,
            org_id: None,
            occurred_at,
            amount: None,
            quantity: None,
            dimensions: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn org(mut self, org: Option<OrgId>) -> Self {
        self.org_id = org;
        self
    }

    #[must_use]
    pub fn amount(mut self, amount: Money) -> Self {
        self.amount = Some(amount);
        self
    }

    #[must_use]
    pub fn quantity(mut self, quantity: i64) -> Self {
        self.quantity = Some(quantity);
        self
    }

    #[must_use]
    pub fn dimension(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.dimensions.insert(name, value.into());
        self
    }
}

/// Function that extracts facts from an event: pure, no I/O (runs inside `reporting`'s event receiver).
pub type Extract = fn(&Delivered) -> Result<Vec<Fact>, Error>;

/// Which facts an event of a module produces (`Module::facts`).
#[derive(Clone)]
pub struct FactSource {
    event: &'static str,
    kinds: Vec<FactKind>,
    extract: Extract,
}

impl FactSource {
    /// `event`: name of the module's OWN event (`<module>.<action>`, declared in `Module::events`).
    pub fn new(event: &'static str, kinds: Vec<FactKind>, extract: Extract) -> Self {
        FactSource { event, kinds, extract }
    }

    pub fn event(&self) -> &'static str {
        self.event
    }

    pub fn kinds(&self) -> &[FactKind] {
        &self.kinds
    }

    /// Extracts facts from `event`, then checks each fact matches the declaration (kind, unit, dimensions). A
    /// wrong declaration is the module author's error: an internal error, the event is redelivered (and shows
    /// up in the log).
    pub fn extract(&self, event: &Delivered) -> Result<Vec<Fact>, Error> {
        let facts = (self.extract)(event)?;
        for fact in &facts {
            self.check(fact)?;
        }
        Ok(facts)
    }

    fn check(&self, fact: &Fact) -> Result<(), Error> {
        let bad = |why: &str| {
            Error::internal(format!(
                "fact `{}` from event `{}` does not match its declaration: {why}",
                fact.kind, self.event
            ))
        };
        let Some(kind) = self.kinds.iter().find(|kind| kind.kind == fact.kind) else {
            return Err(bad("the kind is not declared by this source"));
        };
        match kind.measure {
            Measure::Amount if fact.amount.is_none() => return Err(bad("it has no amount")),
            Measure::Quantity if fact.quantity.is_none() => return Err(bad("it has no quantity")),
            _ => {}
        }
        if let Some(name) = fact.dimensions.keys().find(|name| !kind.dimensions.contains(name)) {
            return Err(bad(&format!("dimension `{name}` is not declared")));
        }
        if fact.dimensions.values().any(|value| value.chars().count() > DIMENSION_VALUE_MAX) {
            return Err(bad("a dimension value is longer than 100 characters"));
        }
        Ok(())
    }
}

impl std::fmt::Debug for FactSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FactSource")
            .field("event", &self.event)
            .field("kinds", &self.kinds)
            .finish_non_exhaustive()
    }
}

/// Maximum length of a dimension value (a short code, not free text).
pub const DIMENSION_VALUE_MAX: usize = 100;

/// Every fact source of every registered module: `(module, source)`, in registration order.
#[derive(Debug, Clone, Default)]
pub struct FactCatalog(std::sync::Arc<Vec<(&'static str, FactSource)>>);

impl FactCatalog {
    /// Validates and collects the modules' sources. `events`: every declared event — a source may only listen
    /// to events of its own module (like `Outbox::record`). Returns all problems at once.
    pub fn new(
        modules: Vec<(&'static str, Vec<FactSource>)>,
        events: &EventCatalog,
    ) -> Result<Self, Vec<String>> {
        let mut problems = Vec::new();
        let mut kinds = BTreeMap::new();
        let mut sources: Vec<(&'static str, FactSource)> = Vec::new();
        for (module, declared) in modules {
            for source in declared {
                // Một event một nguồn: thứ tự số liệu trong event là khoá chống trùng.
                if sources.iter().any(|(_, known)| known.event == source.event) {
                    problems.push(format!(
                        "the facts of event `{}` are declared twice; put every kind in one \
                         FactSource",
                        source.event
                    ));
                }
                problems.extend(source_problems(module, &source, events, &mut kinds));
                sources.push((module, source));
            }
        }
        if problems.is_empty() {
            Ok(FactCatalog(std::sync::Arc::new(sources)))
        } else {
            Err(problems)
        }
    }

    /// `(module, source)` in registration order.
    pub fn sources(&self) -> Vec<(&'static str, FactSource)> {
        self.0.as_ref().clone()
    }
}

fn source_problems(
    module: &'static str,
    source: &FactSource,
    events: &EventCatalog,
    seen: &mut BTreeMap<&'static str, (&'static str, FactKind)>,
) -> Vec<String> {
    let mut problems = Vec::new();
    let own = |name: &str| {
        name.split_once('.')
            .is_some_and(|(prefix, rest)| prefix == module && crate::module::is_identifier(rest))
    };
    if !own(source.event) {
        problems.push(format!(
            "module `{module}` declares facts for event `{}`: a module only reads facts from its \
             own events (`{module}.<event>`)",
            source.event
        ));
    } else if !events.contains(source.event) {
        problems.push(format!(
            "module `{module}` declares facts for event `{}`, which it does not declare in \
             Module::events",
            source.event
        ));
    }
    if source.kinds.is_empty() {
        problems.push(format!("the facts of event `{}` declare no kind", source.event));
    }
    for kind in &source.kinds {
        if !own(kind.kind) {
            problems.push(format!(
                "fact kind `{}` of module `{module}` must be named `{module}.<fact>`, in \
                 lowercase letters, digits and `_`",
                kind.kind
            ));
        }
        if kind.description.trim().is_empty() {
            problems.push(format!("fact kind `{}` has no description", kind.kind));
        }
        // Nhiều event của cùng module được sinh cùng một loại (vd ghi sổ và đảo sổ cùng ra
        // `ledger.revenue`) khi khai giống hệt nhau.
        match seen.get(kind.kind) {
            Some((owner, declared)) if *owner != module || declared != kind => {
                problems.push(format!("fact kind `{}` is declared twice", kind.kind));
            }
            Some(_) => {}
            None => {
                seen.insert(kind.kind, (module, *kind));
            }
        }
        for dimension in kind.dimensions {
            if !crate::module::is_identifier(dimension) {
                problems.push(format!(
                    "dimension `{dimension}` of fact kind `{}` must be lowercase letters, digits \
                     and `_`",
                    kind.kind
                ));
            }
        }
    }
    problems
}

/// The `org_id` node in an event payload (`None` when missing or not a UUID).
pub fn org_field(payload: &Value) -> Option<OrgId> {
    payload
        .get("org_id")
        .and_then(Value::as_str)
        .and_then(|text| Uuid::parse_str(text).ok())
        .map(OrgId::from_uuid)
}

/// The amount in payload field `field` (`{"amount": "1999", "currency": "VND"}`); `None` when
/// the field is absent or `null`. A malformed value is an internal error (the module's own event).
pub fn money_field(payload: &Value, field: &str) -> Result<Option<Money>, Error> {
    match payload.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value(value.clone()).map(Some).map_err(|error| {
            Error::internal(format!("event field `{field}` is not an amount of money: {error}"))
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::EventInfo;
    use crate::money::Currency;
    use crate::tenancy::test_tenant;

    const TOTAL: FactKind = FactKind::new("shop.total", "Total.", Measure::Amount, &["kind"]);
    const COUNT: FactKind = FactKind::new("shop.count", "Count.", Measure::Quantity, &[]);

    fn events() -> EventCatalog {
        EventCatalog::new(vec![(
            "shop",
            vec![
                EventInfo::new("shop.sold", "Sold."),
                EventInfo::new("shop.returned", "Returned."),
            ],
        )])
        .unwrap()
    }

    fn sold(event: &Delivered) -> Result<Vec<Fact>, Error> {
        let amount =
            money_field(&event.payload, "total")?.ok_or_else(|| Error::internal("no total"))?;
        Ok(vec![
            Fact::new(TOTAL.kind, event.occurred_at)
                .org(org_field(&event.payload))
                .amount(amount)
                .dimension("kind", "retail"),
            Fact::new(COUNT.kind, event.occurred_at).quantity(1),
        ])
    }

    #[test]
    fn khai_bao_dung_thi_nhan() {
        let source = FactSource::new("shop.sold", vec![TOTAL, COUNT], sold);
        // Cùng loại, khai giống hệt, ở hai event của cùng module: được.
        let again = FactSource::new("shop.returned", vec![COUNT], sold);
        let catalog = FactCatalog::new(vec![("shop", vec![source, again])], &events()).unwrap();
        assert_eq!(catalog.sources().len(), 2);
    }

    #[test]
    fn khai_bao_sai_bi_bao_het_mot_luc() {
        let foreign = FactSource::new("auth.user_created", vec![TOTAL], sold);
        let undeclared = FactSource::new("shop.refunded", vec![COUNT], sold);
        let bad_kind = FactSource::new(
            "shop.sold",
            vec![FactKind::new("other.total", " ", Measure::Amount, &["Bad-Name"])],
            sold,
        );
        let twice = FactSource::new(
            "shop.sold",
            vec![TOTAL, FactKind::new("shop.total", "Other.", Measure::Amount, &[])],
            sold,
        );
        let problems =
            FactCatalog::new(vec![("shop", vec![foreign, undeclared, bad_kind, twice])], &events())
                .unwrap_err();
        // event lạ, event chưa khai, tên loại sai, mô tả rỗng, chiều sai, event khai hai lần, loại
        // khai hai lần khác nhau.
        assert_eq!(problems.len(), 7, "{problems:#?}");
    }

    #[test]
    fn rut_so_lieu_va_kiem_khop_khai_bao() {
        let org = Uuid::now_v7();
        let payload =
            serde_json::json!({"org_id": org, "total": {"amount": "1999", "currency": "VND"}});
        let event = Delivered::new(test_tenant(), "shop", "shop.sold", payload);
        let source = FactSource::new("shop.sold", vec![TOTAL, COUNT], sold);
        let facts = source.extract(&event).unwrap();
        assert_eq!(facts.len(), 2);
        assert_eq!(facts[0].amount, Some(Money::new(1999, Currency::VND)));
        assert_eq!(facts[0].org_id, Some(OrgId::from_uuid(org)));

        // Thiếu khai báo chiều / sai đơn vị: lỗi nội bộ, không âm thầm ghi số liệu sai.
        let narrow = FactSource::new(
            "shop.sold",
            vec![FactKind::new("shop.total", "Total.", Measure::Amount, &[]), COUNT],
            sold,
        );
        assert!(narrow.extract(&event).is_err());
        let wrong = FactSource::new(
            "shop.sold",
            vec![TOTAL, FactKind::new("shop.count", "Count.", Measure::Amount, &[])],
            sold,
        );
        assert!(wrong.extract(&event).is_err());
    }
}

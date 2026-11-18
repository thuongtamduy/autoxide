//! Phương thức giao, bậc phí, điểm nhận tại cửa hàng (bước 44 mục 2). Mỗi phương thức thuộc một nút;
//! đọc / ghi kiểm `shipping.settings.manage` ở nút đó. Vùng ở `settings.rs`.

use std::collections::{BTreeMap, BTreeSet};

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{OrgId, OrgTree, ScopeExt, TenantContext};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter,
    QueryOrder, Set,
};
use uuid::Uuid;

use crate::api::MethodKind;
use crate::entity::{method, pickup_point, rate};
use crate::errors;
use crate::permissions::SETTINGS_MANAGE;
use crate::service::{ShippingService, check_length, now, unique_violation};
use crate::settings::{
    MethodFields, MethodView, PickupInput, RateInput, check_overlaps, default_org, fail_if_any,
    find_method, find_zone, readable, writable,
};

impl ShippingService {
    // ---- Phương thức ----------------------------------------------------------------------

    pub(crate) async fn list_methods(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: Option<Uuid>,
        page: &PageRequest,
    ) -> Result<Page<method::Model>, Error> {
        let mut condition = Condition::all().add(principal.scope_condition(
            &SETTINGS_MANAGE,
            method::Column::OrgId,
            None,
        )?);
        if let Some(org) = org {
            condition = condition.add(method::Column::OrgId.eq(org));
        }
        let select = method::Entity::find().filter(condition).scoped(tenant);
        fetch_page(select, method::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    pub(crate) async fn get_method(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<MethodView, Error> {
        let read = tenant.database().read();
        let method = find_method(read, tenant, id).await?;
        readable(principal, read, &SETTINGS_MANAGE, method.org_id, || errors::method_not_found(id))
            .await?;
        view_of(read, tenant, method).await
    }

    pub(crate) async fn create_method(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        fields: MethodFields,
    ) -> Result<MethodView, Error> {
        let org = match fields.org_id {
            Some(org) => org,
            None => default_org(tenant, principal, &SETTINGS_MANAGE)?.as_uuid(),
        };
        let at = now();
        let blank = method::Model {
            id: Uuid::now_v7(),
            tenant_id: tenant.id().as_uuid(),
            org_id: org,
            code: String::new(),
            kind: String::new(),
            names: "{}".to_owned(),
            active: true,
            position: 0,
            carrier_account_id: None,
            service_code: None,
            free_over_minor: None,
            handling_fee_minor: 0,
            currency: String::new(),
            tax_category: "vat.standard".to_owned(),
            volumetric_divisor: 6000,
            default_item_weight_grams: None,
            countries: "[\"VN\"]".to_owned(),
            estimated_min_days: None,
            estimated_max_days: None,
            calendar_id: None,
            cutoff_minute: None,
            same_day: false,
            hold_days: None,
            auto_book: false,
            created_at: at,
            updated_at: at,
        };
        let row = apply(blank, fields, true)?;
        let transaction = tenant.database().begin_write().await?;
        principal
            .require_write_at(
                &transaction,
                &SETTINGS_MANAGE,
                OrgTarget::new(OrgId::from_uuid(org), None),
            )
            .await?;
        let code = row.code.clone();
        let active: method::ActiveModel = row.into();
        let inserted = match active.insert(&transaction).await {
            Ok(row) => row,
            Err(error) if unique_violation(&error).is_some() => {
                return Err(errors::method_code_taken(&code));
            }
            Err(error) => return Err(error.into()),
        };
        transaction.commit().await?;
        Ok(MethodView { method: inserted, rates: Vec::new(), pickup_points: Vec::new() })
    }

    pub(crate) async fn update_method(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        fields: MethodFields,
    ) -> Result<MethodView, Error> {
        let transaction = tenant.database().begin_write().await?;
        let current = find_method(&transaction, tenant, id).await?;
        writable(principal, &transaction, &SETTINGS_MANAGE, current.org_id, || {
            errors::method_not_found(id)
        })
        .await?;
        let mut row = apply(current, fields, false)?;
        row.updated_at = now();
        let mut active: method::ActiveModel = row.into();
        // Mọi cột coi như đã đổi: `apply` đã dựng lại toàn bộ dòng.
        active = active.reset_all();
        active.id = sea_orm::ActiveValue::Unchanged(id);
        let row = active.update(&transaction).await?;
        let view = view_of(&transaction, tenant, row).await?;
        transaction.commit().await?;
        Ok(view)
    }

    pub(crate) async fn delete_method(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        let transaction = tenant.database().begin_write().await?;
        let current = find_method(&transaction, tenant, id).await?;
        writable(principal, &transaction, &SETTINGS_MANAGE, current.org_id, || {
            errors::method_not_found(id)
        })
        .await?;
        rate::Entity::delete_many()
            .filter(rate::Column::MethodId.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        pickup_point::Entity::delete_many()
            .filter(pickup_point::Column::MethodId.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        method::Entity::delete_many()
            .filter(method::Column::Id.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Thay toàn bộ bậc phí của phương thức. Vùng phải đọc được; bậc chồng nhau bị từ chối.
    pub(crate) async fn put_rates(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        rates: Vec<RateInput>,
    ) -> Result<MethodView, Error> {
        let mut problems = Vec::new();
        for (index, input) in rates.iter().enumerate() {
            if input.min_weight_grams < 0 {
                problems.push(FieldError::new(
                    format!("rates[{index}].min_weight_grams"),
                    "range",
                    "must be 0 or more",
                ));
            }
            if input.max_weight_grams.is_some_and(|max| max <= input.min_weight_grams) {
                problems.push(FieldError::new(
                    format!("rates[{index}].max_weight_grams"),
                    "range",
                    "must be more than `min_weight_grams`",
                ));
            }
            if input.max_side_mm.is_some_and(|side| side <= 0) {
                problems.push(FieldError::new(
                    format!("rates[{index}].max_side_mm"),
                    "range",
                    "must be more than 0",
                ));
            }
            if input.amount.is_negative() {
                problems.push(FieldError::new(
                    format!("rates[{index}].amount"),
                    "range",
                    "must be 0 or more",
                ));
            }
            if input.step_grams.is_some() != input.step_amount.is_some()
                || input.step_grams.is_some_and(|step| step <= 0)
            {
                problems.push(FieldError::new(
                    format!("rates[{index}].step_grams"),
                    "step",
                    "give both `step_grams` (more than 0) and `step_amount`, or neither",
                ));
            }
        }
        fail_if_any(problems)?;
        check_overlaps(&rates)?;

        let transaction = tenant.database().begin_write().await?;
        let method = find_method(&transaction, tenant, id).await?;
        writable(principal, &transaction, &SETTINGS_MANAGE, method.org_id, || {
            errors::method_not_found(id)
        })
        .await?;
        let mut problems = Vec::new();
        for (index, input) in rates.iter().enumerate() {
            for (field, money) in
                [("amount", Some(input.amount)), ("step_amount", input.step_amount)]
            {
                if let Some(money) = money
                    && money.currency().code() != method.currency
                {
                    problems.push(
                        FieldError::new(
                            format!("rates[{index}].{field}"),
                            "currency",
                            "must be in {currency}",
                        )
                        .with_param("currency", &method.currency),
                    );
                }
            }
        }
        fail_if_any(problems)?;
        let zone_ids: BTreeSet<Uuid> = rates.iter().map(|input| input.zone_id).collect();
        for zone_id in &zone_ids {
            let zone = find_zone(&transaction, tenant, *zone_id).await?;
            readable(principal, &transaction, &SETTINGS_MANAGE, zone.org_id, || {
                errors::zone_not_found(*zone_id)
            })
            .await?;
        }
        rate::Entity::delete_many()
            .filter(rate::Column::MethodId.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        for input in &rates {
            rate::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                method_id: Set(id),
                zone_id: Set(input.zone_id),
                min_weight_grams: Set(input.min_weight_grams),
                max_weight_grams: Set(input.max_weight_grams),
                max_side_mm: Set(input.max_side_mm),
                amount_minor: Set(input.amount.minor()),
                currency: Set(method.currency.clone()),
                step_grams: Set(input.step_grams),
                step_amount_minor: Set(input.step_amount.map(Money::minor)),
            }
            .insert(&transaction)
            .await?;
        }
        let view = view_of(&transaction, tenant, method).await?;
        transaction.commit().await?;
        Ok(view)
    }

    /// Thay toàn bộ điểm nhận của phương thức nhận tại cửa hàng. Mỗi cửa hàng phải thuộc nhánh của
    /// nút sở hữu phương thức.
    pub(crate) async fn put_pickup_points(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        points: Vec<PickupInput>,
    ) -> Result<MethodView, Error> {
        let mut problems = Vec::new();
        let mut seen = BTreeSet::new();
        for (index, point) in points.iter().enumerate() {
            check_length(
                &mut problems,
                &format!("pickup_points[{index}].instructions"),
                &point.instructions,
                0,
                2000,
            );
            if !(0..=720).contains(&point.prep_hours) {
                problems.push(FieldError::new(
                    format!("pickup_points[{index}].prep_hours"),
                    "range",
                    "must be 0 to 720",
                ));
            }
            if !seen.insert(point.org_id) {
                problems.push(FieldError::new(
                    format!("pickup_points[{index}].org_id"),
                    "duplicate",
                    "is listed twice",
                ));
            }
        }
        fail_if_any(problems)?;
        let transaction = tenant.database().begin_write().await?;
        let method = find_method(&transaction, tenant, id).await?;
        writable(principal, &transaction, &SETTINGS_MANAGE, method.org_id, || {
            errors::method_not_found(id)
        })
        .await?;
        if method.kind != MethodKind::Pickup.as_str() {
            return Err(errors::not_pickup_method());
        }
        let owner = OrgId::from_uuid(method.org_id);
        let mut problems = Vec::new();
        for (index, point) in points.iter().enumerate() {
            let store = OrgId::from_uuid(point.org_id);
            if store != owner
                && !OrgTree::is_within(&transaction, tenant.id(), store, &[owner]).await?
            {
                problems.push(FieldError::new(
                    format!("pickup_points[{index}].org_id"),
                    "org_outside",
                    "must be the unit of the method or a unit below it",
                ));
            }
        }
        fail_if_any(problems)?;
        pickup_point::Entity::delete_many()
            .filter(pickup_point::Column::MethodId.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        for point in &points {
            pickup_point::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                method_id: Set(id),
                org_id: Set(point.org_id),
                instructions: Set(point.instructions.trim().to_owned()),
                prep_hours: Set(point.prep_hours),
            }
            .insert(&transaction)
            .await?;
        }
        let view = view_of(&transaction, tenant, method).await?;
        transaction.commit().await?;
        Ok(view)
    }
}

/// Áp `fields` lên `row` và kiểm toàn bộ dòng. `creating`: `code`, `kind`, `names`, `currency`
/// bắt buộc; khi sửa thì không đổi được `code`, `kind`, `org_id`, `currency`.
fn apply(
    mut row: method::Model,
    fields: MethodFields,
    creating: bool,
) -> Result<method::Model, Error> {
    let mut problems = Vec::new();
    let immutable = |problems: &mut Vec<FieldError>, field: &str| {
        problems.push(FieldError::new(
            field,
            "immutable",
            "cannot be changed; create another method",
        ));
    };
    match (creating, fields.code) {
        (true, Some(code)) => {
            let code = code.trim().to_owned();
            if !autoxide_kernel::slug::is_valid(&code) || code.len() > 64 {
                problems.push(FieldError::new(
                    "code",
                    "code",
                    "must be 1 to 64 lower-case letters, digits and `-`, like `standard`",
                ));
            }
            row.code = code;
        }
        (true, None) => problems.push(FieldError::new("code", "required", "is required")),
        (false, Some(_)) => immutable(&mut problems, "code"),
        (false, None) => {}
    }
    match (creating, fields.kind) {
        (true, Some(kind)) => row.kind = kind.as_str().to_owned(),
        (true, None) => problems.push(FieldError::new("kind", "required", "is required")),
        (false, Some(_)) => immutable(&mut problems, "kind"),
        (false, None) => {}
    }
    match (creating, fields.currency) {
        (true, Some(code)) => match Currency::parse(code.trim()) {
            Ok(currency) => row.currency = currency.code().to_owned(),
            Err(_) => problems.push(FieldError::new(
                "currency",
                "currency",
                "must be an ISO 4217 code like `VND`",
            )),
        },
        (true, None) => problems.push(FieldError::new("currency", "required", "is required")),
        (false, Some(_)) => immutable(&mut problems, "currency"),
        (false, None) => {}
    }
    if !creating && fields.org_id.is_some() {
        immutable(&mut problems, "org_id");
    }
    if let Some(names) = fields.names {
        let names: BTreeMap<String, String> = names
            .into_iter()
            .map(|(locale, name)| (locale.trim().to_owned(), name.trim().to_owned()))
            .filter(|(locale, _)| !locale.is_empty())
            .collect();
        for (locale, name) in &names {
            check_length(&mut problems, &format!("names.{locale}"), name, 1, 200);
        }
        row.names = serde_json::to_string(&names).unwrap_or_else(|_| "{}".to_owned());
    }
    if row.names == "{}" {
        problems.push(FieldError::new(
            "names",
            "required",
            "give the name in at least one language",
        ));
    }
    if let Some(active) = fields.active {
        row.active = active;
    }
    if let Some(position) = fields.position {
        row.position = position;
    }
    if let Some(account) = fields.carrier_account_id {
        row.carrier_account_id = account;
    }
    if let Some(service) = fields.service_code {
        row.service_code =
            service.map(|code| code.trim().to_owned()).filter(|code| !code.is_empty());
    }
    let currency = row.currency.clone();
    let money = |problems: &mut Vec<FieldError>, field: &str, value: Money| -> i64 {
        if value.currency().code() != currency && !currency.is_empty() {
            problems.push(
                FieldError::new(field, "currency", "must be in {currency}")
                    .with_param("currency", &currency),
            );
        }
        if value.is_negative() {
            problems.push(FieldError::new(field, "range", "must be 0 or more"));
        }
        value.minor()
    };
    if let Some(free_over) = fields.free_over {
        row.free_over_minor = free_over.map(|value| money(&mut problems, "free_over", value));
    }
    if let Some(fee) = fields.handling_fee {
        row.handling_fee_minor = money(&mut problems, "handling_fee", fee);
    }
    if let Some(category) = fields.tax_category {
        let category = category.trim().to_owned();
        check_length(&mut problems, "tax_category", &category, 1, 64);
        row.tax_category = category;
    }
    if let Some(divisor) = fields.volumetric_divisor {
        if !(1000..=10_000).contains(&divisor) {
            problems.push(FieldError::new("volumetric_divisor", "range", "must be 1000 to 10000"));
        }
        row.volumetric_divisor = divisor;
    }
    if let Some(weight) = fields.default_item_weight_grams {
        if weight.is_some_and(|grams| grams <= 0) {
            problems.push(FieldError::new(
                "default_item_weight_grams",
                "range",
                "must be more than 0",
            ));
        }
        row.default_item_weight_grams = weight;
    }
    if let Some(countries) = fields.countries {
        let mut codes = BTreeSet::new();
        for (index, code) in countries.iter().enumerate() {
            let code = code.trim().to_ascii_uppercase();
            if code.len() != 2 || !code.bytes().all(|byte| byte.is_ascii_uppercase()) {
                problems.push(FieldError::new(
                    format!("countries[{index}]"),
                    "country",
                    "must be a two-letter country code",
                ));
            }
            codes.insert(code);
        }
        if codes.is_empty() {
            problems.push(FieldError::new("countries", "required", "list at least one country"));
        }
        row.countries = serde_json::to_string(&codes).unwrap_or_else(|_| "[\"VN\"]".to_owned());
    }
    if let Some(days) = fields.estimated_days {
        if let Some((min, max)) = days
            && (min < 0 || max < min || max > 365)
        {
            problems.push(FieldError::new(
                "estimated_days",
                "range",
                "must be 0 to 365 days, the minimum first",
            ));
        }
        row.estimated_min_days = days.map(|(min, _)| min);
        row.estimated_max_days = days.map(|(_, max)| max);
    }
    if let Some(calendar) = fields.calendar_id {
        row.calendar_id = calendar;
    }
    if let Some(cutoff) = fields.cutoff_minute {
        if cutoff.is_some_and(|minute| !(0..1440).contains(&minute)) {
            problems.push(FieldError::new(
                "cutoff_minute",
                "range",
                "must be 0 to 1439 (minutes after midnight)",
            ));
        }
        row.cutoff_minute = cutoff;
    }
    if let Some(same_day) = fields.same_day {
        row.same_day = same_day;
    }
    if let Some(hold) = fields.hold_days {
        if hold.is_some_and(|days| !(1..=60).contains(&days)) {
            problems.push(FieldError::new("hold_days", "range", "must be 1 to 60"));
        }
        row.hold_days = hold;
    }
    if let Some(auto_book) = fields.auto_book {
        row.auto_book = auto_book;
    }
    if row.kind == MethodKind::Carrier.as_str() && row.carrier_account_id.is_none() {
        problems.push(FieldError::new(
            "carrier_account_id",
            "required",
            "is required for a `carrier` method",
        ));
    }
    fail_if_any(problems)?;
    Ok(row)
}

async fn view_of<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    method: method::Model,
) -> Result<MethodView, Error> {
    let rates = rate::Entity::find()
        .filter(rate::Column::MethodId.eq(method.id))
        .scoped(tenant)
        .order_by_asc(rate::Column::ZoneId)
        .order_by_asc(rate::Column::MinWeightGrams)
        .all(connection)
        .await?;
    let pickup_points = pickup_point::Entity::find()
        .filter(pickup_point::Column::MethodId.eq(method.id))
        .scoped(tenant)
        .order_by_asc(pickup_point::Column::Id)
        .all(connection)
        .await?;
    Ok(MethodView { method, rates, pickup_points })
}

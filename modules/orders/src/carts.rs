//! Giỏ hàng (mục 2 của bước 45): người mua đăng nhập có một giỏ đang dùng ở mỗi nút bán; người mua
//! vãng lai giữ giỏ bằng `Cart-Token` (32 byte ngẫu nhiên, trả một lần, database chỉ giữ bản băm).
//! Giỏ chỉ lưu biến thể + số lượng và lựa chọn của người mua; giá, giảm, phí giao, thuế luôn tính
//! lại ở máy chủ mỗi lần đọc (`pricing::quote`).

use autoxide_address::api::AddressInput;
use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::money::Currency;
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::secret::{random_token, sha256};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext, root_of};
use autoxide_promotions::api::{Eligibility, GiftSelection};
use autoxide_shipping::api::Address;
use chrono::{DateTime, Duration, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, Set,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::{Attribution, BuyerProfile};
use crate::entity::{cart, cart_line};
use crate::errors;
use crate::pricing::{self, PriceInput, Quote};
use crate::service::OrdersService;

/// Trạng thái của giỏ.
pub(crate) const ACTIVE: &str = "active";
pub(crate) const MERGED: &str = "merged";
pub(crate) const CONVERTED: &str = "converted";
pub(crate) const ABANDONED: &str = "abandoned";

/// Ai đang dùng giỏ.
#[derive(Debug, Clone)]
pub(crate) enum Owner {
    Buyer(Uuid),
    /// `Cart-Token` thô người mua gửi.
    Guest(String),
}

impl Owner {
    /// Người mua đăng nhập (`principal`), không thì vãng lai với `Cart-Token`.
    pub(crate) fn of(principal: Option<&Principal>, token: Option<&str>) -> Option<Owner> {
        match (principal, token) {
            (Some(principal), _) => Some(Owner::Buyer(principal.user_id())),
            (None, Some(token)) if !token.trim().is_empty() => {
                Some(Owner::Guest(token.trim().to_owned()))
            }
            _ => None,
        }
    }

    fn buyer(&self) -> Option<Uuid> {
        match self {
            Owner::Buyer(user) => Some(*user),
            Owner::Guest(_) => None,
        }
    }
}

/// Nơi giao của giỏ: một địa chỉ trong sổ của người mua, hay địa chỉ nhập tay (vãng lai).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ShipTo {
    pub address_id: Option<Uuid>,
    pub recipient_name: Option<String>,
    pub phone: Option<String>,
    pub address: Option<Address>,
}

/// Giỏ đã đọc, kèm dòng và báo giá tính lại.
#[derive(Debug, Clone)]
pub(crate) struct CartView {
    pub cart: cart::Model,
    pub codes: Vec<String>,
    pub voucher_ids: Vec<Uuid>,
    pub ship_to: Option<ShipTo>,
    pub attribution: Option<Attribution>,
    pub quote: Quote,
}

/// Giỏ vừa tạo; `token` chỉ có với giỏ vãng lai mới.
pub(crate) struct NewCart {
    pub view: CartView,
    pub token: Option<String>,
}

fn json_list<T: for<'de> Deserialize<'de>>(raw: &str) -> Vec<T> {
    serde_json::from_str(raw).unwrap_or_default()
}

fn to_json<T: Serialize>(value: &T) -> Result<String, Error> {
    serde_json::to_string(value).map_err(Error::internal)
}

fn buyer_key(org: Uuid, buyer: Uuid) -> String {
    format!("{org}:{buyer}")
}

impl OrdersService {
    /// Tạo giỏ ở nút bán `org` (mặc định: nút gốc của khách). Người mua đăng nhập đã có giỏ đang
    /// dùng ở nút đó thì trả lại giỏ đó (`Replayed`).
    pub(crate) async fn create_cart(
        &self,
        tenant: &TenantContext,
        principal: Option<&Principal>,
        org: Option<Uuid>,
        currency: Option<&str>,
        locale: Option<&str>,
    ) -> Result<Submitted<NewCart>, Error> {
        let org = org.map_or_else(|| root_of(tenant.id()), OrgId::from_uuid);
        let currency = Currency::parse(currency.unwrap_or("VND"))?;
        let locale = locale.unwrap_or("vi").to_owned();
        let buyer = principal.map(Principal::user_id);
        if let Some(buyer) = buyer
            && let Some(existing) = self.active_cart_of(tenant, org, buyer).await?
        {
            let view = self.view(tenant, existing, &Owner::Buyer(buyer)).await?;
            return Ok(Submitted::Replayed(NewCart { view, token: None }));
        }
        let now = crate::service::now();
        let token = if buyer.is_none() { Some(random_token(32)?) } else { None };
        let row = cart::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(org.as_uuid()),
            buyer_user_id: Set(buyer),
            token_hash: Set(token.as_deref().map(sha256)),
            currency: Set(currency.code().to_owned()),
            locale: Set(locale),
            codes: Set("[]".to_owned()),
            voucher_ids: Set("[]".to_owned()),
            gift_selections: Set("[]".to_owned()),
            ship_to: Set(None),
            shipping_option: Set(None),
            payment_method: Set(None),
            attribution: Set(None),
            status: Set(ACTIVE.to_owned()),
            merged_into: Set(None),
            active_buyer_key: Set(buyer.map(|buyer| buyer_key(org.as_uuid(), buyer))),
            abandoned_notified_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            expires_at: Set(now + Duration::days(self.config.cart_ttl_days)),
        };
        let inserted = match row.insert(tenant.database().write()).await {
            Ok(inserted) => inserted,
            // Hai request cùng lúc của cùng người mua: bên sau thấy giỏ của bên trước.
            Err(error) if buyer.is_some() => {
                let Some(buyer) = buyer else { return Err(error.into()) };
                let existing = self
                    .active_cart_of(tenant, org, buyer)
                    .await?
                    .ok_or_else(|| Error::from(error))?;
                let view = self.view(tenant, existing, &Owner::Buyer(buyer)).await?;
                return Ok(Submitted::Replayed(NewCart { view, token: None }));
            }
            Err(error) => return Err(error.into()),
        };
        let owner = match (&token, buyer) {
            (Some(token), _) => Owner::Guest(token.clone()),
            (None, Some(buyer)) => Owner::Buyer(buyer),
            (None, None) => return Err(Error::internal("a cart has neither buyer nor token")),
        };
        let view = self.view(tenant, inserted, &owner).await?;
        Ok(Submitted::Created(NewCart { view, token }))
    }

    async fn active_cart_of(
        &self,
        tenant: &TenantContext,
        org: OrgId,
        buyer: Uuid,
    ) -> Result<Option<cart::Model>, Error> {
        Ok(cart::Entity::find()
            .filter(cart::Column::ActiveBuyerKey.eq(buyer_key(org.as_uuid(), buyer)))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?)
    }

    /// Giỏ `id` của `owner`. Không có, của khách khác hay hết hạn → 404; sai token → 401.
    pub(crate) async fn owned_cart(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        owner: &Owner,
    ) -> Result<cart::Model, Error> {
        let row = cart::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .filter(|row| row.expires_at > crate::service::now())
            .ok_or_else(errors::cart_not_found)?;
        let owned = match owner {
            Owner::Buyer(user) => row.buyer_user_id == Some(*user),
            Owner::Guest(token) => {
                row.buyer_user_id.is_none() && row.token_hash.as_deref() == Some(&sha256(token))
            }
        };
        if !owned {
            return Err(match owner {
                Owner::Buyer(_) => errors::cart_not_found(),
                Owner::Guest(_) => errors::cart_token_invalid(),
            });
        }
        Ok(row)
    }

    /// Giỏ đang dùng được sửa (không phải giỏ đã gộp / đã thành đơn).
    async fn editable_cart(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        owner: &Owner,
    ) -> Result<cart::Model, Error> {
        let row = self.owned_cart(tenant, id, owner).await?;
        if row.status == MERGED || row.status == CONVERTED {
            return Err(errors::cart_closed());
        }
        Ok(row)
    }

    /// Giỏ kèm dòng và báo giá tính lại ở máy chủ.
    pub(crate) async fn view(
        &self,
        tenant: &TenantContext,
        row: cart::Model,
        owner: &Owner,
    ) -> Result<CartView, Error> {
        let lines = cart_line::Entity::find()
            .filter(cart_line::Column::CartId.eq(row.id))
            .scoped(tenant)
            .order_by_asc(cart_line::Column::AddedAt)
            .order_by_asc(cart_line::Column::VariantId)
            .all(tenant.database().read())
            .await?;
        let input = self.price_input(tenant, &row, owner, &lines).await?;
        let quote =
            pricing::quote(&self.sources, tenant, &input, self.config.quote_ttl_minutes).await?;
        Ok(CartView {
            codes: json_list(&row.codes),
            voucher_ids: json_list(&row.voucher_ids),
            ship_to: row.ship_to.as_deref().and_then(|raw| serde_json::from_str(raw).ok()),
            attribution: row.attribution.as_deref().and_then(|raw| serde_json::from_str(raw).ok()),
            quote,
            cart: row,
        })
    }

    /// Đầu vào báo giá của giỏ: dòng, lựa chọn của người mua, hồ sơ nhóm khách (bước 47 cắm vào).
    pub(crate) async fn price_input(
        &self,
        tenant: &TenantContext,
        row: &cart::Model,
        owner: &Owner,
        lines: &[cart_line::Model],
    ) -> Result<PriceInput, Error> {
        let buyer = owner.buyer();
        let profile = match (buyer, self.extensions.buyer_groups()) {
            (Some(buyer), Some(groups)) => groups.profile_of(tenant, buyer).await?,
            _ => BuyerProfile::default(),
        };
        let ship_to: Option<ShipTo> =
            row.ship_to.as_deref().and_then(|raw| serde_json::from_str(raw).ok());
        let destination = match (&ship_to, buyer, &self.address_book) {
            (Some(ShipTo { address: Some(address), .. }), _, _) => Some(address.clone()),
            (Some(ShipTo { address_id: Some(id), .. }), Some(buyer), Some(book)) => {
                book.get(tenant, buyer, *id).await.ok().map(|found| found.address)
            }
            _ => None,
        };
        let mut buyer_keys = Vec::new();
        if let Some(buyer) = buyer {
            buyer_keys.push(format!("user:{buyer}"));
        }
        Ok(PriceInput {
            org_id: OrgId::from_uuid(row.org_id),
            currency: Currency::parse(&row.currency)?,
            locale: row.locale.clone(),
            lines: lines.iter().map(|line| (line.variant_id, line.quantity)).collect(),
            codes: json_list(&row.codes),
            voucher_ids: json_list(&row.voucher_ids),
            gift_selections: json_list::<GiftSelection>(&row.gift_selections),
            buyer_id: buyer,
            buyer_keys,
            profile,
            destination,
            shipping_option: row.shipping_option.clone(),
            payment_method: row.payment_method.clone(),
            client: Some("web".to_owned()),
            first_order: false,
            at: crate::service::now(),
        })
    }

    /// Đặt số lượng của biến thể `variant` trong giỏ; 0 xoá dòng. Biến thể ngừng bán → 422; hàng
    /// chỉ bán khi còn mà không đủ → 409 (chỉ báo, chưa giữ chỗ).
    pub(crate) async fn set_line(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        owner: &Owner,
        variant: Uuid,
        quantity: i64,
    ) -> Result<CartView, Error> {
        let max = self.config.line_max_quantity;
        if !(0..=max).contains(&quantity) {
            return Err(errors::quantity_invalid(max));
        }
        let row = self.editable_cart(tenant, id, owner).await?;
        if quantity > 0 {
            self.check_sellable(tenant, &row, owner, variant, quantity).await?;
        }
        let max_lines = self.config.cart_max_lines;
        let ttl_days = self.config.cart_ttl_days;
        let database = tenant.database();
        let (tenant_id, cart_id) = (tenant.id().as_uuid(), row.id);
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let existing = cart_line::Entity::find_by_id((tenant_id, cart_id, variant))
                    .one(&transaction)
                    .await?;
                let now = crate::service::now();
                match (existing, quantity) {
                    (Some(line), 0) => {
                        cart_line::Entity::delete_by_id((
                            line.tenant_id,
                            line.cart_id,
                            line.variant_id,
                        ))
                        .exec(&transaction)
                        .await?;
                    }
                    (Some(line), _) => {
                        let mut line: cart_line::ActiveModel = line.into();
                        line.quantity = Set(quantity);
                        line.updated_at = Set(now);
                        line.update(&transaction).await?;
                    }
                    (None, 0) => {}
                    (None, _) => {
                        let count = cart_line::Entity::find()
                            .filter(cart_line::Column::TenantId.eq(tenant_id))
                            .filter(cart_line::Column::CartId.eq(cart_id))
                            .count(&transaction)
                            .await?;
                        if count >= max_lines as u64 {
                            return Err(errors::cart_too_many_lines(max_lines));
                        }
                        cart_line::ActiveModel {
                            tenant_id: Set(tenant_id),
                            cart_id: Set(cart_id),
                            variant_id: Set(variant),
                            quantity: Set(quantity),
                            added_at: Set(now),
                            updated_at: Set(now),
                        }
                        .insert(&transaction)
                        .await?;
                    }
                }
                touch(&transaction, tenant_id, cart_id, ttl_days).await?;
                transaction.commit().await?;
                Ok(())
            })
            .await?;
        self.reload(tenant, id, owner).await
    }

    /// Biến thể bán được với số lượng này (chỉ báo, chưa giữ chỗ).
    async fn check_sellable(
        &self,
        tenant: &TenantContext,
        row: &cart::Model,
        owner: &Owner,
        variant: Uuid,
        quantity: i64,
    ) -> Result<(), Error> {
        let mut input = self.price_input(tenant, row, owner, &[]).await?;
        input.lines = vec![(variant, quantity)];
        input.codes.clear();
        input.voucher_ids.clear();
        input.destination = None;
        let probe = pricing::Sources {
            promotions: None,
            shipping: None,
            tax: None,
            ..self.sources.clone()
        };
        let quote = pricing::quote(&probe, tenant, &input, 1).await?;
        match quote.lines.first().and_then(pricing::line_error) {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    async fn reload(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        owner: &Owner,
    ) -> Result<CartView, Error> {
        let row = self.owned_cart(tenant, id, owner).await?;
        self.view(tenant, row, owner).await
    }

    /// Ghi một thay đổi lựa chọn của giỏ (`edit` sửa dòng giỏ), rồi trả giỏ tính lại.
    async fn edit_cart(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        owner: &Owner,
        edit: impl FnOnce(&mut cart::ActiveModel),
    ) -> Result<CartView, Error> {
        let row = self.editable_cart(tenant, id, owner).await?;
        let mut active: cart::ActiveModel = row.into();
        edit(&mut active);
        let now = crate::service::now();
        active.updated_at = Set(now);
        active.expires_at = Set(now + Duration::days(self.config.cart_ttl_days));
        active.abandoned_notified_at = Set(None);
        if active.status.as_ref() == ABANDONED {
            active.status = Set(ACTIVE.to_owned());
        }
        active.update(tenant.database().write()).await?;
        self.reload(tenant, id, owner).await
    }

    /// Thay danh sách coupon của giỏ. Mã không dùng được không làm lỗi: báo giá trả kèm lý do.
    pub(crate) async fn set_codes(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        owner: &Owner,
        codes: Vec<String>,
    ) -> Result<CartView, Error> {
        let mut clean: Vec<String> = Vec::new();
        let mut problems = Vec::new();
        for (index, code) in codes.iter().enumerate() {
            let code = code.trim().to_uppercase();
            if code.is_empty() || code.len() > 64 {
                problems.push(FieldError::new(
                    format!("codes[{index}]"),
                    "invalid",
                    "must be 1 to 64 characters long",
                ));
            } else if !clean.contains(&code) {
                clean.push(code);
            }
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let raw = to_json(&clean)?;
        self.edit_cart(tenant, id, owner, |row| row.codes = Set(raw)).await
    }

    /// Thay danh sách voucher trong ví dùng cho giỏ (chỉ người đăng nhập — D-222). Voucher không
    /// thuộc ví của người mua bị `promotions` từ chối kèm lý do trong báo giá.
    pub(crate) async fn set_vouchers(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        owner: &Owner,
        vouchers: Vec<Uuid>,
    ) -> Result<CartView, Error> {
        if owner.buyer().is_none() {
            return Err(errors::vouchers_need_account());
        }
        let mut clean = vouchers;
        clean.sort();
        clean.dedup();
        let raw = to_json(&clean)?;
        self.edit_cart(tenant, id, owner, |row| row.voucher_ids = Set(raw)).await
    }

    /// Đặt nơi giao và cách giao. `address_id` chỉ cho người đăng nhập (sổ địa chỉ của `shipping`);
    /// vãng lai gửi địa chỉ (đã kiểm ở `address` khi module có cài).
    pub(crate) async fn set_delivery(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        owner: &Owner,
        delivery: Delivery,
    ) -> Result<CartView, Error> {
        let mut problems = Vec::new();
        let ship_to = match (delivery.address_id, delivery.address) {
            (Some(_), Some(_)) => {
                problems.push(FieldError::new(
                    "address",
                    "conflict",
                    "send either `address_id` or `address`, not both",
                ));
                None
            }
            (Some(address_id), None) => {
                match (owner.buyer(), &self.address_book) {
                    (Some(buyer), Some(book)) => {
                        book.get(tenant, buyer, address_id).await?;
                    }
                    _ => problems.push(FieldError::new(
                        "address_id",
                        "not_allowed",
                        "is only for signed-in buyers; send `address` instead",
                    )),
                }
                Some(ShipTo {
                    address_id: Some(address_id),
                    recipient_name: None,
                    phone: None,
                    address: None,
                })
            }
            (None, Some(input)) => {
                let address = match &self.units {
                    Some(units) => match units.validate(input, "address").await {
                        Ok(address) => Some(address),
                        Err(fields) => {
                            problems.extend(fields);
                            None
                        }
                    },
                    None => Some(unchecked_address(input)?),
                };
                Some(ShipTo {
                    address_id: None,
                    recipient_name: delivery.recipient_name,
                    phone: delivery.phone,
                    address,
                })
            }
            (None, None) => None,
        };
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let ship_to = ship_to.map(|ship_to| to_json(&ship_to)).transpose()?;
        let option = delivery.shipping_option;
        self.edit_cart(tenant, id, owner, |row| {
            row.ship_to = Set(ship_to);
            row.shipping_option = Set(option);
        })
        .await
    }

    /// Chọn phương thức thanh toán (khuyến mại theo phương thức thanh toán tính lại — D-222).
    pub(crate) async fn set_payment_method(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        owner: &Owner,
        method: Option<String>,
    ) -> Result<CartView, Error> {
        self.edit_cart(tenant, id, owner, |row| row.payment_method = Set(method)).await
    }

    /// Ghi nguồn giới thiệu (tiếp thị liên kết, livestream — bước 49).
    pub(crate) async fn set_attribution(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        owner: &Owner,
        attribution: Attribution,
    ) -> Result<CartView, Error> {
        let raw = to_json(&attribution)?;
        self.edit_cart(tenant, id, owner, |row| row.attribution = Set(Some(raw))).await
    }

    /// Mã / voucher dùng được và chưa đủ điều kiện cho giỏ (D-201, D-222, D-228).
    pub(crate) async fn cart_promotions(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        owner: &Owner,
    ) -> Result<Option<Eligibility>, Error> {
        let Some(promotions) = &self.sources.promotions else { return Ok(None) };
        let view = self.reload(tenant, id, owner).await?;
        let lines = cart_line::Entity::find()
            .filter(cart_line::Column::CartId.eq(id))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        let input = self.price_input(tenant, &view.cart, owner, &lines).await?;
        let cart = pricing::promotion_cart_of(&input, &view.quote);
        Ok(Some(promotions.eligible(tenant, &cart).await?))
    }

    /// Gộp giỏ vãng lai `guest_id` (chứng minh bằng `token`) vào giỏ đang dùng của người mua `buyer`
    /// ở cùng nút bán: cộng số lượng, chặn ở trần; mã giảm giá gộp. Giỏ vãng lai thành `merged`.
    /// Gọi lại sau khi đã gộp trả giỏ của người mua, không cộng lần hai.
    pub(crate) async fn merge(
        &self,
        tenant: &TenantContext,
        buyer: Uuid,
        guest_id: Uuid,
        token: &str,
    ) -> Result<CartView, Error> {
        let guest = Owner::Guest(token.to_owned());
        let source = self.owned_cart(tenant, guest_id, &guest).await?;
        let org = OrgId::from_uuid(source.org_id);
        let target = match self.active_cart_of(tenant, org, buyer).await? {
            Some(target) => target,
            None => {
                let created = self
                    .create_cart(
                        tenant,
                        Some(&Principal::customer(buyer)),
                        Some(source.org_id),
                        Some(&source.currency),
                        Some(&source.locale),
                    )
                    .await?;
                created.into_inner().view.cart
            }
        };
        if source.status == MERGED {
            return self.view(tenant, target, &Owner::Buyer(buyer)).await;
        }
        let max = self.config.line_max_quantity;
        let max_lines = self.config.cart_max_lines;
        let database = tenant.database();
        let tenant_id = tenant.id().as_uuid();
        let (source_id, target_id) = (source.id, target.id);
        let source_codes: Vec<String> = json_list(&source.codes);
        let target_codes: Vec<String> = json_list(&target.codes);
        let (source_codes, target_codes) = (&source_codes, &target_codes);
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                // Khoá dòng giỏ nguồn: hai lệnh gộp cùng lúc, bên sau thấy `merged`.
                let Some(current) = autoxide_kernel::rows::lock_one(
                    &transaction,
                    cart::Entity::find_by_id(source_id)
                        .filter(cart::Column::TenantId.eq(tenant_id)),
                )
                .await?
                else {
                    return Err(errors::cart_not_found());
                };
                if current.status == MERGED {
                    return Ok(());
                }
                let lines = cart_line::Entity::find()
                    .filter(cart_line::Column::TenantId.eq(tenant_id))
                    .filter(cart_line::Column::CartId.eq(source_id))
                    .all(&transaction)
                    .await?;
                let mut count = cart_line::Entity::find()
                    .filter(cart_line::Column::TenantId.eq(tenant_id))
                    .filter(cart_line::Column::CartId.eq(target_id))
                    .count(&transaction)
                    .await?;
                let now = crate::service::now();
                for line in lines {
                    let existing =
                        cart_line::Entity::find_by_id((tenant_id, target_id, line.variant_id))
                            .one(&transaction)
                            .await?;
                    match existing {
                        Some(found) => {
                            let quantity = found.quantity.saturating_add(line.quantity).min(max);
                            let mut found: cart_line::ActiveModel = found.into();
                            found.quantity = Set(quantity);
                            found.updated_at = Set(now);
                            found.update(&transaction).await?;
                        }
                        // Giỏ đích đã đầy: bỏ dòng thừa (người mua thấy giỏ sau khi gộp).
                        None if count >= max_lines as u64 => {}
                        None => {
                            count += 1;
                            cart_line::ActiveModel {
                                tenant_id: Set(tenant_id),
                                cart_id: Set(target_id),
                                variant_id: Set(line.variant_id),
                                quantity: Set(line.quantity.min(max)),
                                added_at: Set(now),
                                updated_at: Set(now),
                            }
                            .insert(&transaction)
                            .await?;
                        }
                    }
                }
                let mut codes = target_codes.clone();
                for code in source_codes {
                    if !codes.contains(code) {
                        codes.push(code.clone());
                    }
                }
                cart::Entity::update_many()
                    .col_expr(cart::Column::Codes, Expr::value(to_json(&codes)?))
                    .col_expr(cart::Column::UpdatedAt, Expr::value(now))
                    .filter(cart::Column::TenantId.eq(tenant_id))
                    .filter(cart::Column::Id.eq(target_id))
                    .exec(&transaction)
                    .await?;
                cart::Entity::update_many()
                    .col_expr(cart::Column::Status, Expr::value(MERGED))
                    .col_expr(cart::Column::MergedInto, Expr::value(target_id))
                    .col_expr(cart::Column::UpdatedAt, Expr::value(now))
                    .filter(cart::Column::TenantId.eq(tenant_id))
                    .filter(cart::Column::Id.eq(source_id))
                    .exec(&transaction)
                    .await?;
                transaction.commit().await?;
                Ok(())
            })
            .await?;
        self.reload(tenant, target_id, &Owner::Buyer(buyer)).await
    }
}

/// Đầu vào của `PUT /carts/{id}/delivery`.
#[derive(Debug, Clone, Default)]
pub(crate) struct Delivery {
    pub address_id: Option<Uuid>,
    pub address: Option<AddressInput>,
    pub recipient_name: Option<String>,
    pub phone: Option<String>,
    pub shipping_option: Option<String>,
}

/// Địa chỉ không kiểm được (chưa cài `address`): giữ mã, tên để trống.
fn unchecked_address(input: AddressInput) -> Result<Address, Error> {
    // `Address` là `#[non_exhaustive]` ở crate khác: dựng qua JSON.
    serde_json::from_value(serde_json::json!({
        "country": input.country, "province_code": input.province_code, "province_name": "",
        "commune_code": input.commune_code, "commune_name": "", "line": input.line,
    }))
    .map_err(Error::internal)
}

/// Đổi `updated_at` / `expires_at` của giỏ khi dòng đổi.
async fn touch(
    connection: &impl ConnectionTrait,
    tenant_id: Uuid,
    cart_id: Uuid,
    ttl_days: i64,
) -> Result<(), Error> {
    let now: DateTime<Utc> = crate::service::now();
    cart::Entity::update_many()
        .col_expr(cart::Column::Status, Expr::value(ACTIVE))
        .col_expr(cart::Column::UpdatedAt, Expr::value(now))
        .col_expr(cart::Column::ExpiresAt, Expr::value(now + Duration::days(ttl_days)))
        .col_expr(cart::Column::AbandonedNotifiedAt, Expr::value(Option::<DateTime<Utc>>::None))
        .filter(cart::Column::TenantId.eq(tenant_id))
        .filter(cart::Column::Id.eq(cart_id))
        .exec(connection)
        .await?;
    Ok(())
}

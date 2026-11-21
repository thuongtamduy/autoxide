//! Tính khuyến mại cho một giỏ ở mức service: kiểm giỏ, mở mã / voucher, lấy biến thể từ
//! `Catalog`, chạy bộ tính thuần, cắt trần pháp luật và sàn giá (giá tham chiếu, sàn giá từ
//! `Catalog` — ngoài mọi transaction), chia người trả tiền, dịch lý do, dấu vân tay. Không ghi gì.

use autoxide_kernel::time::Timestamp;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use autoxide_catalog::api::{FloorAction, PriceQuery};
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::money::Money;
use autoxide_kernel::tenancy::{OrgId, OrgTree, ScopeExt, TenantContext};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::json;
use uuid::Uuid;

use crate::api::{
    AppliedPromotion, Cart, ClawbackMode, Evaluation, FundingShare, GiftLine, LineDiscount,
    MAX_CART_LINES, RejectedCode, ShippingDiscount, WithheldDiscount,
};
use crate::compiled::Compiled;
use crate::compliance::legal_day;
use crate::engine::caps::{self, LineCap};
use crate::engine::{self, Applied, Draft, Input, ReasonDraft, Rejection, Unlock, VariantFacts};
use crate::entity::{code, voucher};
use crate::legal::legal_kind;
use crate::model::{Program, Purpose, Status, Trigger};
use crate::reasons::render;
use crate::service::PromotionsService;

/// Tuỳ chọn của một lần tính.
#[derive(Debug, Clone, Default)]
pub(crate) struct Options {
    /// Chạy thử: tính cả chương trình này (kể cả `draft`), thay bản trong bộ nhớ nếu có.
    pub include: Option<Arc<Program>>,
    /// Chạy thử "không có chương trình": bỏ chương trình này.
    pub exclude: Option<Uuid>,
    /// Chỉ giảm tự động lớp `product` không điều kiện giỏ (giá công khai, D-200).
    pub unconditional_only: bool,
}

/// Chương trình giảm tự động lớp `product` không phụ thuộc giỏ: giá sau giảm của một đơn vị là
/// giá công khai được (D-200, D-208…D-218).
pub(crate) fn unconditional(program: &Program) -> bool {
    use crate::model::{Kind, Scope};
    program.trigger == Trigger::Automatic
        && program.scope == Scope::Items
        && matches!(
            program.kind,
            Kind::PercentOff | Kind::AmountOff | Kind::FixedPrice | Kind::FlashSale
        )
        && program.min_order_minor.is_none()
        && program.min_quantity.is_none()
        && program.max_discount_minor.is_none()
        && program.budget_minor.is_none()
        && program.customer_groups.is_empty()
        && program.channels.is_empty()
        && !program.first_order_only
        && program.conditions.is_empty()
}

/// Kết quả tính kèm phần nội bộ cho việc giữ lượt.
pub(crate) struct Outcome {
    pub evaluation: Evaluation,
    /// Chương trình đã áp, theo thứ tự áp, kèm chi tiết bộ tính.
    pub applied: Vec<(Arc<Program>, Applied)>,
    /// Dòng của giỏ (theo vị trí) — để biết biến thể của từng phần giảm.
    pub cart: Cart,
}

/// Khoá so mã: chữ hoa, bỏ khoảng trắng.
pub(crate) fn code_key(code: &str) -> String {
    code.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_uppercase()
}

impl PromotionsService {
    /// Kiểm hình dạng của giỏ.
    pub(crate) fn check_cart(&self, cart: &Cart) -> Result<(), Error> {
        if cart.codes.len() > self.config.max_codes_per_cart {
            return Err(Error::new(
                ErrorKind::Validation,
                "promotions.too_many_codes",
                "A cart may use at most {max} codes.",
            )
            .with_param("max", self.config.max_codes_per_cart));
        }
        if cart.lines.len() > MAX_CART_LINES {
            return Err(Error::validation(vec![FieldError::new(
                "lines",
                "invalid",
                "a cart has at most 500 lines",
            )]));
        }
        if cart.voucher_ids.len() > 20
            || cart.gift_selections.len() > 50
            || cart.buyer_keys.len() > 10
        {
            return Err(Error::validation(vec![FieldError::new(
                "voucher_ids",
                "invalid",
                "too many vouchers, gift selections or buyer keys",
            )]));
        }
        let mut keys = BTreeSet::new();
        for line in &cart.lines {
            if !keys.insert(line.line_key.as_str())
                || line.line_key.is_empty()
                || line.line_key.len() > 100
            {
                return Err(Error::validation(vec![FieldError::new(
                    "lines",
                    "invalid",
                    "each `line_key` is 1 to 100 characters and unique",
                )]));
            }
            if !(1..=1_000_000).contains(&line.quantity) {
                return Err(Error::validation(vec![FieldError::new(
                    "lines",
                    "invalid",
                    "each quantity is 1 to 1000000",
                )]));
            }
            if line.unit_price.currency() != cart.currency || line.unit_price.is_negative() {
                return Err(Error::validation(vec![FieldError::new(
                    "lines",
                    "invalid",
                    "each unit price is in the cart's currency, not negative",
                )]));
            }
        }
        if cart.shipping_fee.is_some_and(|fee| fee.currency() != cart.currency || fee.is_negative())
        {
            return Err(Error::validation(vec![FieldError::new(
                "shipping_fee",
                "invalid",
                "the shipping fee is in the cart's currency",
            )]));
        }
        if cart.buyer_keys.iter().any(|key| key.is_empty() || key.len() > 128) {
            return Err(Error::validation(vec![FieldError::new(
                "buyer_keys",
                "invalid",
                "each buyer key is 1 to 128 characters",
            )]));
        }
        Ok(())
    }

    /// Tính khuyến mãi của giỏ cho nhân viên (REST `POST /evaluate` và tool MCP `evaluate_cart`):
    /// `promotions.read` phải phủ nút bán của giỏ — không xem được chương trình của nhánh khác.
    pub(crate) async fn evaluate_for(
        &self,
        tenant: &TenantContext,
        principal: &autoxide_kernel::access::Principal,
        cart: &Cart,
    ) -> Result<Outcome, Error> {
        principal
            .require_at(
                tenant.database().read(),
                &crate::permissions::READ,
                crate::service::at(cart.org_id.as_uuid()),
            )
            .await?;
        self.evaluate_cart(tenant, cart, &Options::default()).await
    }

    /// Tính khuyến mại cho giỏ.
    pub(crate) async fn evaluate_cart(
        &self,
        tenant: &TenantContext,
        cart: &Cart,
        options: &Options,
    ) -> Result<Outcome, Error> {
        self.check_cart(cart)?;
        let compiled = self
            .compiled
            .get(tenant, std::time::Duration::from_secs(self.config.compiled_ttl_seconds))
            .await?;
        let read = tenant.database().read();
        let org = cart.org_id;
        let mut ancestors: Vec<Uuid> = OrgTree::ancestors(read, tenant.id(), org)
            .await?
            .into_iter()
            .map(|id| id.as_uuid())
            .collect();
        if ancestors.is_empty() {
            ancestors.push(org.as_uuid());
        }
        let zone = self.timezone(tenant, org).await?;
        let best = self
            .setting(tenant, org, crate::settings::STACKING_MODE.key())
            .await?
            .is_some_and(|value| value.as_str() == Some("best_for_buyer"));
        let clawback =
            match self.setting(tenant, org, crate::settings::CLAWBACK_ON_RETURN.key()).await? {
                Some(value) if value.as_str() == Some("proportional") => ClawbackMode::Proportional,
                _ => ClawbackMode::None,
            };
        // Biến thể (sản phẩm, danh mục kèm tổ tiên, người bán) — trước mọi transaction.
        let variant_ids: Vec<Uuid> = cart
            .lines
            .iter()
            .map(|line| line.variant_id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut variants = HashMap::new();
        for chunk in variant_ids.chunks(autoxide_catalog::api::MAX_IDS) {
            for found in self.catalog.variants(tenant, chunk, None).await? {
                variants.insert(
                    found.variant_id,
                    VariantFacts {
                        product_id: found.product_id,
                        org_id: found.org_id.as_uuid(),
                        category_ids: found.category_ids.clone(),
                    },
                );
            }
        }
        let (unlocked, mut rejected) = self.unlock(tenant, cart, &compiled, options).await?;
        // Ứng viên: tra chỉ mục cho các dòng của giỏ, cộng chương trình đã mở bằng mã / voucher.
        let mut programs: Vec<&Program> = compiled.candidates(cart.lines.iter().map(|line| {
            let facts = variants.get(&line.variant_id);
            (
                line.variant_id,
                facts.map(|facts| facts.product_id),
                facts.map_or(&[][..], |facts| facts.category_ids.as_slice()),
            )
        }));
        for id in unlocked.keys() {
            if let Some(program) = compiled.get(*id)
                && !programs.iter().any(|found| found.id == *id)
            {
                programs.push(program);
            }
        }
        if let Some(include) = &options.include {
            programs.retain(|found| found.id != include.id);
            programs.push(include.as_ref());
        }
        if let Some(exclude) = options.exclude {
            programs.retain(|found| found.id != exclude);
        }
        if options.unconditional_only {
            programs.retain(|found| unconditional(found));
        }
        let input = Input {
            cart,
            variants: &variants,
            ancestors: &ancestors,
            local: cart.at.in_zone(&zone).datetime(),
            best_for_buyer: best,
            max_vouchers_per_class: self.config.max_vouchers_per_class,
            unlocked: &unlocked,
            include_inactive: options.include.as_ref().map(|program| program.id),
        };
        let mut draft = engine::compute(&programs, &input);
        draft.rejected.append(&mut rejected);
        let by_id: HashMap<Uuid, Arc<Program>> =
            programs.iter().map(|program| (program.id, Arc::new((*program).clone()))).collect();
        let withheld = self.apply_caps(tenant, cart, &by_id, &mut draft).await?;
        let evaluation = self.present(cart, &by_id, &draft, &withheld, clawback)?;
        let applied = draft
            .applied
            .into_iter()
            .filter_map(|applied| Some((by_id.get(&applied.program)?.clone(), applied)))
            .collect();
        Ok(Outcome { evaluation, applied, cart: cart.clone() })
    }

    /// Mở chương trình bằng mã / voucher của giỏ; mã / voucher sai thành lý do, không lỗi.
    async fn unlock(
        &self,
        tenant: &TenantContext,
        cart: &Cart,
        compiled: &Compiled,
        options: &Options,
    ) -> Result<(HashMap<Uuid, Unlock>, Vec<Rejection>), Error> {
        let mut unlocked: HashMap<Uuid, Unlock> = HashMap::new();
        let mut rejected = Vec::new();
        let running = |id: Uuid| -> Option<&Program> {
            match &options.include {
                Some(include) if include.id == id => Some(include.as_ref()),
                _ => compiled.get(id).filter(|program| program.status == Status::Active),
            }
        };
        let reject = |code: Option<String>, voucher: Option<Uuid>, reason: &'static str| {
            Rejection { program: None, code, voucher_id: voucher, reason: ReasonDraft::new(reason) }
        };
        let read = tenant.database().read();
        if !cart.codes.is_empty() {
            let keys: Vec<String> = cart.codes.iter().map(|found| code_key(found)).collect();
            let rows = code::Entity::find()
                .filter(code::Column::CodeKey.is_in(keys))
                .scoped(tenant)
                .all(read)
                .await?;
            let mut seen = BTreeSet::new();
            for entered in &cart.codes {
                let key = code_key(entered);
                if !seen.insert(key.clone()) {
                    continue;
                }
                let found = rows.iter().find(|row| row.code_key == key);
                let Some(row) = found.filter(|row| row.purpose == Purpose::Redeem.as_str()) else {
                    rejected.push(reject(
                        Some(entered.clone()),
                        None,
                        "promotions.reason.code_invalid",
                    ));
                    continue;
                };
                let program =
                    running(row.promotion_id).filter(|program| program.trigger == Trigger::Code);
                let Some(program) = program else {
                    rejected.push(reject(
                        Some(entered.clone()),
                        None,
                        "promotions.reason.code_expired",
                    ));
                    continue;
                };
                if row.usage_limit.is_some_and(|limit| row.usage_count >= limit) {
                    rejected.push(reject(
                        Some(entered.clone()),
                        None,
                        "promotions.reason.usage_exhausted",
                    ));
                    continue;
                }
                if unlocked.contains_key(&program.id) {
                    rejected.push(reject(
                        Some(entered.clone()),
                        None,
                        "promotions.reason.not_combinable",
                    ));
                    continue;
                }
                unlocked.insert(
                    program.id,
                    Unlock {
                        code: Some(row.code.clone()),
                        code_id: Some(row.id),
                        voucher_id: None,
                    },
                );
            }
        }
        if !cart.voucher_ids.is_empty() {
            let rows = voucher::Entity::find()
                .filter(voucher::Column::Id.is_in(cart.voucher_ids.clone()))
                .scoped(tenant)
                .all(read)
                .await?;
            let mut seen = BTreeSet::new();
            for id in &cart.voucher_ids {
                if !seen.insert(*id) {
                    continue;
                }
                let reason = match (rows.iter().find(|row| row.id == *id), cart.buyer_id) {
                    (_, None) => Some("promotions.reason.sign_in_required"),
                    (None, _) => Some("promotions.reason.voucher_not_owned"),
                    (Some(row), Some(buyer)) if row.holder_user_id != Some(buyer) => {
                        Some("promotions.reason.voucher_not_owned")
                    }
                    (Some(row), _) if row.status == "held" || row.status == "used" => {
                        Some("promotions.reason.voucher_used")
                    }
                    (Some(row), _)
                        if row.status != "available"
                            || cart.at < row.valid_from
                            || cart.at >= row.expires_at =>
                    {
                        Some("promotions.reason.voucher_expired")
                    }
                    (Some(row), _) => match running(row.promotion_id) {
                        Some(program) if program.trigger == Trigger::Voucher => {
                            match unlocked.entry(program.id) {
                                std::collections::hash_map::Entry::Occupied(_) => {
                                    Some("promotions.reason.not_combinable")
                                }
                                std::collections::hash_map::Entry::Vacant(slot) => {
                                    slot.insert(Unlock {
                                        code: None,
                                        code_id: None,
                                        voucher_id: Some(*id),
                                    });
                                    None
                                }
                            }
                        }
                        _ => Some("promotions.reason.voucher_expired"),
                    },
                };
                if let Some(reason) = reason {
                    rejected.push(reject(None, Some(*id), reason));
                }
            }
        }
        Ok((unlocked, rejected))
    }

    /// Trần pháp luật (giá tham chiếu × hạn mức đang hiệu lực) và sàn giá `block` cho mỗi dòng có
    /// giảm.
    async fn apply_caps(
        &self,
        tenant: &TenantContext,
        cart: &Cart,
        programs: &HashMap<Uuid, Arc<Program>>,
        draft: &mut Draft,
    ) -> Result<Vec<caps::Withheld>, Error> {
        let lines: BTreeSet<usize> = draft
            .applied
            .iter()
            .flat_map(|applied| applied.lines.iter().map(|part| part.line))
            .collect();
        if lines.is_empty() {
            return Ok(Vec::new());
        }
        let legal = self.legal().await?;
        let day = legal_day(cart.at);
        let mut caps_by_line = vec![LineCap::default(); cart.lines.len()];
        let mut missing = vec![false; cart.lines.len()];
        // Mỗi dòng: thời điểm bắt đầu sớm nhất và trần thấp nhất của các chương trình đã áp.
        let mut instants: BTreeMap<Timestamp, BTreeSet<Uuid>> = BTreeMap::new();
        let mut line_info: HashMap<usize, (Timestamp, Option<i32>)> = HashMap::new();
        for applied in &draft.applied {
            let Some(program) = programs.get(&applied.program) else { continue };
            let max = legal
                .limit(&self.config.jurisdiction, legal_kind(program), program.program_kind, day)
                .map(|limit| limit.max_bp);
            for part in &applied.lines {
                let entry = line_info.entry(part.line).or_insert((program.starts_at, max));
                entry.0 = entry.0.min(program.starts_at);
                entry.1 = match (entry.1, max) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                };
            }
        }
        for (line, (instant, _)) in &line_info {
            instants.entry(*instant).or_default().insert(cart.lines[*line].variant_id);
        }
        let mut references: HashMap<(Timestamp, Uuid), Money> = HashMap::new();
        for (instant, variants) in &instants {
            let query = PriceQuery::new(
                cart.org_id,
                cart.currency,
                *instant,
                variants.iter().copied().collect(),
            );
            for found in self.catalog.reference_prices(tenant, &query).await? {
                if found.amount.currency() == cart.currency {
                    references.insert((*instant, found.variant_id), found.amount);
                }
            }
        }
        let discounted: Vec<Uuid> = lines
            .iter()
            .map(|line| cart.lines[*line].variant_id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let floors =
            self.catalog.price_floors(tenant, cart.org_id, cart.currency, &discounted).await?;
        for (line, (instant, max)) in &line_info {
            let cart_line = &cart.lines[*line];
            if let Some(max) = max {
                match references.get(&(*instant, cart_line.variant_id)) {
                    Some(reference) => {
                        let cap = i128::from(reference.minor())
                            * i128::from(cart_line.quantity)
                            * i128::from(*max)
                            / 10_000;
                        caps_by_line[*line].law = Some(i64::try_from(cap).unwrap_or(i64::MAX));
                    }
                    None => {
                        // Chưa có giá ngay trước khuyến mại: không áp được trần → không giảm.
                        caps_by_line[*line].law = Some(0);
                        missing[*line] = true;
                    }
                }
            }
            if let Some(floor) =
                floors.iter().find(|floor| floor.variant_id == cart_line.variant_id)
                && floor.action == FloorAction::Block
                && let Some(minimum) = floor.floor
            {
                let room = (cart_line.unit_price.minor() - minimum.minor()).max(0);
                caps_by_line[*line].floor = Some(room.saturating_mul(cart_line.quantity));
            }
        }
        Ok(caps::apply(draft, &caps_by_line, &missing))
    }

    /// Dựng [`Evaluation`] từ kết quả bộ tính: chia người trả tiền, dịch lý do, dấu vân tay.
    fn present(
        &self,
        cart: &Cart,
        programs: &HashMap<Uuid, Arc<Program>>,
        draft: &Draft,
        withheld: &[caps::Withheld],
        clawback: ClawbackMode,
    ) -> Result<Evaluation, Error> {
        let currency = cart.currency;
        let locale = cart.locale.as_str();
        let reason = |draft: &ReasonDraft| render(&self.translations, draft, locale);
        let mut lines = Vec::new();
        let mut shipping = Vec::new();
        let mut gifts = Vec::new();
        let mut applied_list = Vec::new();
        for applied in &draft.applied {
            let Some(program) = programs.get(&applied.program) else { continue };
            let program_reason = reason(&applied.reason);
            let mut parts: Vec<&engine::LinePart> = applied.lines.iter().collect();
            parts.sort_by(|a, b| cart.lines[a.line].line_key.cmp(&cart.lines[b.line].line_key));
            for part in parts {
                let amount = Money::new(part.amount, currency);
                lines.push(LineDiscount {
                    line_key: cart.lines[part.line].line_key.clone(),
                    promotion_id: program.id,
                    class: program.class(),
                    amount,
                    reason: program_reason.clone(),
                    capped_by_law: part.capped_by_law,
                    capped_by_floor: part.capped_by_floor,
                    funding: funding(program, Some(cart.lines[part.line].variant_id), amount)?,
                });
            }
            if applied.shipping > 0 {
                let amount = Money::new(applied.shipping, currency);
                shipping.push(ShippingDiscount {
                    promotion_id: program.id,
                    amount,
                    reason: program_reason.clone(),
                    funding: funding(program, None, amount)?,
                });
            }
            for (variant, quantity) in &applied.gifts {
                gifts.push(GiftLine {
                    promotion_id: program.id,
                    variant_id: *variant,
                    quantity: *quantity,
                    reason: program_reason.clone(),
                });
            }
            let (title, terms) = program.text(locale);
            applied_list.push(AppliedPromotion {
                promotion_id: program.id,
                code: applied.unlock.as_ref().and_then(|unlock| unlock.code.clone()),
                voucher_id: applied.unlock.as_ref().and_then(|unlock| unlock.voucher_id),
                class: program.class(),
                title,
                terms,
                amount: Money::new(applied.total(), currency),
                min_order: program.min_order_minor.map(|minor| Money::new(minor, currency)),
                gift_return_policy: program.gift_return_policy.api(),
                return_usage_on_cancel: program.return_usage_on_cancel,
            });
        }
        let mut rejected: Vec<RejectedCode> = draft
            .rejected
            .iter()
            .map(|found| RejectedCode {
                code: found.code.clone(),
                voucher_id: found.voucher_id,
                reason: reason(&found.reason),
            })
            .collect();
        rejected.sort_by(|a, b| (&a.code, a.voucher_id).cmp(&(&b.code, b.voucher_id)));
        rejected.dedup_by(|a, b| a.code == b.code && a.voucher_id == b.voucher_id);
        let withheld: Vec<WithheldDiscount> = withheld
            .iter()
            .map(|found| WithheldDiscount {
                promotion_id: found.program,
                line_key: cart.lines[found.line].line_key.clone(),
                reason: reason(&found.reason),
            })
            .collect();
        let shipping_total: i64 = shipping.iter().map(|found| found.amount.minor()).sum();
        let line_total: i64 = lines.iter().map(|found| found.amount.minor()).sum();
        let mut evaluation = Evaluation {
            currency,
            lines,
            shipping_discount: Money::new(shipping_total, currency),
            shipping,
            gifts,
            gift_choices: draft.choices.clone(),
            total_discount: Money::new(line_total + shipping_total, currency),
            applied: applied_list,
            rejected_codes: rejected,
            withheld,
            clawback_on_return: clawback,
            fingerprint: String::new(),
        };
        evaluation.fingerprint = fingerprint(cart, &evaluation);
        Ok(evaluation)
    }
}

/// Chia `amount` cho người trả tiền theo `share_bp` (`Money::allocate`, tổng khớp từng đồng).
fn funding(
    program: &Program,
    variant: Option<Uuid>,
    amount: Money,
) -> Result<Vec<FundingShare>, Error> {
    // Sản phẩm người bán góp vào chương trình của sàn: phần giảm do chính người bán trả.
    if let Some(seller) = variant.and_then(|variant| program.sponsors.get(&variant)) {
        return Ok(vec![FundingShare {
            payer_kind: crate::api::PayerKind::Seller,
            payer_org_id: Some(OrgId::from_uuid(*seller)),
            amount,
        }]);
    }
    if program.funding.is_empty() {
        return Ok(vec![FundingShare {
            payer_kind: crate::api::PayerKind::Seller,
            payer_org_id: Some(OrgId::from_uuid(program.org_id)),
            amount,
        }]);
    }
    let ratios: Vec<u64> =
        program.funding.iter().map(|share| u64::try_from(share.share_bp).unwrap_or(0)).collect();
    let shares = amount.allocate(&ratios)?;
    Ok(program
        .funding
        .iter()
        .zip(shares)
        .map(|(share, amount)| FundingShare {
            payer_kind: share.payer.api(),
            payer_org_id: share.payer_org_id.map(OrgId::from_uuid),
            amount,
        })
        .collect())
}

/// Dấu vân tay: giỏ chuẩn hoá (dòng theo `line_key`, mã chữ hoa, voucher sắp xếp) và kết quả
/// (chương trình đã áp, số tiền) — không có `at` (D-222).
pub(crate) fn fingerprint(cart: &Cart, evaluation: &Evaluation) -> String {
    autoxide_kernel::secret::sha256(&canonical_cart(cart, Some(evaluation)))
}

/// Dạng chuẩn của giỏ (và kết quả, nếu có) để băm.
pub(crate) fn canonical_cart(cart: &Cart, evaluation: Option<&Evaluation>) -> String {
    let mut lines: Vec<_> = cart
        .lines
        .iter()
        .map(|line| {
            json!([
                line.line_key,
                line.variant_id,
                line.quantity,
                line.unit_price.minor(),
                line.has_compare_at
            ])
        })
        .collect();
    lines.sort_by_key(|value| value[0].as_str().map(str::to_owned));
    let mut codes: Vec<String> = cart.codes.iter().map(|found| code_key(found)).collect();
    codes.sort();
    codes.dedup();
    let mut vouchers = cart.voucher_ids.clone();
    vouchers.sort();
    vouchers.dedup();
    let mut gifts: Vec<_> = cart
        .gift_selections
        .iter()
        .map(|gift| json!([gift.promotion_id, gift.variant_id, gift.quantity]))
        .collect();
    gifts.sort_by_key(|value| value.to_string());
    let mut groups = cart.customer_groups.clone();
    groups.sort();
    let result = evaluation.map(|evaluation| {
        let mut discounts: Vec<_> = evaluation
            .lines
            .iter()
            .map(|line| json!([line.promotion_id, line.line_key, line.amount.minor()]))
            .collect();
        discounts.sort_by_key(|value| value.to_string());
        json!({
            "applied": evaluation.applied.iter().map(|found| json!([found.promotion_id, found.amount.minor()])).collect::<Vec<_>>(),
            "lines": discounts,
            "shipping": evaluation.shipping_discount.minor(),
            "gifts": evaluation.gifts.iter().map(|gift| json!([gift.promotion_id, gift.variant_id, gift.quantity])).collect::<Vec<_>>(),
        })
    });
    json!({
        "org": cart.org_id.as_uuid(),
        "currency": cart.currency.code(),
        "buyer": cart.buyer_id,
        "groups": groups,
        "tier": cart.membership_tier,
        "channel": cart.channel,
        "client": cart.client,
        "payment": cart.payment_method,
        "carrier": cart.carrier,
        "province": cart.ship_to_province_code,
        "first_order": cart.first_order,
        "lines": lines,
        "shipping_fee": cart.shipping_fee.map(Money::minor),
        "codes": codes,
        "vouchers": vouchers,
        "gifts": gifts,
        "result": result,
    })
    .to_string()
}

//! Cài đặt `dyn Promotions` cho module khác (`orders` bước 45…): mỗi hàm gọi đúng hàm service mà
//! REST và MCP dùng. Thêm: `eligible` (bộ chọn mã của website, D-201) và `automatic_prices` (giá
//! sau giảm tự động cho danh sách công khai, D-200).

use autoxide_kernel::time::Timestamp;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::money::Money;
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use uuid::Uuid;

use crate::api::{
    AutomaticPrice, Cart, CartLine, Eligibility, Evaluation, GrantVoucher, IneligibleOffer, Offer,
    Promotions, Redemption, ReserveRequest, Selection, Voucher, VoucherStatus,
};
use crate::entity::{code, promotion};
use crate::evaluate::Options;
use crate::model::{Purpose, Status, Trigger};
use crate::reasons::render;
use crate::service::PromotionsService;

/// Bọc service để công bố trait.
pub(crate) struct Provider(pub Arc<PromotionsService>);

/// Số coupon `public` tối đa đưa vào bộ chọn cho mỗi chương trình.
const CODES_PER_PROGRAM: u64 = 3;

impl PromotionsService {
    /// Mã `public`, voucher trong ví dùng được và chưa đủ điều kiện cho giỏ, cùng tổ hợp lợi nhất.
    pub(crate) async fn eligible(
        &self,
        tenant: &TenantContext,
        cart: &Cart,
    ) -> Result<Eligibility, Error> {
        self.check_cart(cart)?;
        let mut base = cart.clone();
        base.codes.clear();
        base.voucher_ids.clear();
        let baseline = self.evaluate_cart(tenant, &base, &Options::default()).await?.evaluation;
        // Ứng viên: coupon `public` của chương trình `code` đang chạy, voucher trong ví.
        let read = tenant.database().read();
        let programs = promotion::Entity::find()
            .filter(promotion::Column::Status.eq(Status::Active.as_str()))
            .filter(promotion::Column::Public.eq(true))
            .filter(promotion::Column::Trigger.eq(Trigger::Code.as_str()))
            .scoped(tenant)
            .all(read)
            .await?;
        let mut candidates: Vec<(Option<String>, Option<Uuid>)> = Vec::new();
        for program in &programs {
            let codes = code::Entity::find()
                .filter(code::Column::PromotionId.eq(program.id))
                .filter(code::Column::Purpose.eq(Purpose::Redeem.as_str()))
                .scoped(tenant)
                .order_by_asc(code::Column::Id)
                .limit(CODES_PER_PROGRAM)
                .all(read)
                .await?;
            candidates.extend(codes.into_iter().map(|found| (Some(found.code), None)));
        }
        if let Some(buyer) = cart.buyer_id {
            for voucher in self.vouchers_of(tenant, buyer, Some(VoucherStatus::Available)).await? {
                candidates.push((None, Some(voucher.id)));
            }
        }
        let mut usable = Vec::new();
        let mut ineligible = Vec::new();
        for (code, voucher_id) in candidates {
            let mut trial = base.clone();
            trial.codes = code.iter().cloned().collect();
            trial.voucher_ids = voucher_id.iter().copied().collect();
            let result = self.evaluate_cart(tenant, &trial, &Options::default()).await?.evaluation;
            let unlocked = result.applied.iter().find(|found| {
                found.code.is_some() == code.is_some()
                    && (found.code.is_some() || found.voucher_id == voucher_id)
            });
            match unlocked {
                Some(applied) => usable.push(Offer {
                    promotion_id: applied.promotion_id,
                    code: code.clone(),
                    voucher_id,
                    class: applied.class,
                    title: applied.title.clone(),
                    terms: applied.terms.clone(),
                    discount: Money::new(
                        result.total_discount.minor() - baseline.total_discount.minor(),
                        cart.currency,
                    ),
                    remaining_uses: None,
                    ends_at: Timestamp::MAX,
                }),
                None => {
                    let reason = result
                        .rejected_codes
                        .iter()
                        .find(|found| found.code == code && found.voucher_id == voucher_id)
                        .map(|found| found.reason.clone())
                        .unwrap_or_else(|| {
                            render(
                                &self.translations,
                                &crate::engine::ReasonDraft::new(
                                    "promotions.reason.not_applicable",
                                ),
                                &cart.locale,
                            )
                        });
                    let promotion_id =
                        self.offer_promotion(tenant, code.as_deref(), voucher_id).await?;
                    let Some(promotion_id) = promotion_id else { continue };
                    ineligible.push(IneligibleOffer {
                        offer: Offer {
                            promotion_id,
                            code: code.clone(),
                            voucher_id,
                            class: crate::api::PromotionClass::Order,
                            title: String::new(),
                            terms: String::new(),
                            discount: Money::zero(cart.currency),
                            remaining_uses: None,
                            ends_at: Timestamp::MAX,
                        },
                        reason,
                    });
                }
            }
        }
        self.fill_offers(tenant, cart, &mut usable, &mut ineligible).await?;
        usable.sort_by(|a, b| {
            b.discount.minor().cmp(&a.discount.minor()).then(a.promotion_id.cmp(&b.promotion_id))
        });
        // Tổ hợp lợi nhất: thêm lần lượt theo mức giảm, giữ khi tổng tăng (tất định).
        let mut selection = Selection::default();
        let mut best = baseline.total_discount;
        for offer in &usable {
            let mut trial = base.clone();
            trial.codes = selection.codes.clone();
            trial.voucher_ids = selection.voucher_ids.clone();
            trial.codes.extend(offer.code.iter().cloned());
            trial.voucher_ids.extend(offer.voucher_id.iter().copied());
            if trial.codes.len() > self.config.max_codes_per_cart {
                continue;
            }
            let total = self
                .evaluate_cart(tenant, &trial, &Options::default())
                .await?
                .evaluation
                .total_discount;
            if total.minor() > best.minor() {
                best = total;
                selection.codes = trial.codes;
                selection.voucher_ids = trial.voucher_ids;
            }
        }
        Ok(Eligibility { usable, ineligible, best_selection: selection, best_total_discount: best })
    }

    /// Chương trình của một mã / voucher.
    async fn offer_promotion(
        &self,
        tenant: &TenantContext,
        code_text: Option<&str>,
        voucher_id: Option<Uuid>,
    ) -> Result<Option<Uuid>, Error> {
        let read = tenant.database().read();
        if let Some(text) = code_text {
            return Ok(code::Entity::find()
                .filter(code::Column::CodeKey.eq(crate::evaluate::code_key(text)))
                .scoped(tenant)
                .one(read)
                .await?
                .map(|found| found.promotion_id));
        }
        match voucher_id {
            Some(id) => Ok(crate::entity::voucher::Entity::find_by_id(id)
                .scoped(tenant)
                .one(read)
                .await?
                .map(|found| found.promotion_id)),
            None => Ok(None),
        }
    }

    /// Điền tiêu đề, điều kiện, hạn và số lượt còn (số thật, đọc từ bộ đếm) của các mã.
    async fn fill_offers(
        &self,
        tenant: &TenantContext,
        cart: &Cart,
        usable: &mut [Offer],
        ineligible: &mut [IneligibleOffer],
    ) -> Result<(), Error> {
        let ids: BTreeSet<Uuid> = usable
            .iter()
            .map(|offer| offer.promotion_id)
            .chain(ineligible.iter().map(|found| found.offer.promotion_id))
            .collect();
        let read = tenant.database().read();
        let rows = promotion::Entity::find()
            .filter(promotion::Column::Id.is_in(ids.iter().copied().collect::<Vec<_>>()))
            .scoped(tenant)
            .all(read)
            .await?;
        let views = crate::programs::load_children(read, tenant, rows).await?;
        let programs: HashMap<Uuid, crate::model::Program> =
            views.iter().map(|view| (view.row.id, crate::compiled::program_of(view))).collect();
        let fill = |offer: &mut Offer| {
            if let Some(program) = programs.get(&offer.promotion_id) {
                let (title, terms) = program.text(&cart.locale);
                offer.title = title;
                offer.terms = terms;
                offer.class = program.class();
                offer.ends_at = program.ends_at;
                offer.remaining_uses = program.remaining_uses();
            }
        };
        usable.iter_mut().for_each(fill);
        ineligible.iter_mut().for_each(|found| fill(&mut found.offer));
        Ok(())
    }

    /// Giá sau giảm tự động lớp `product` không điều kiện giỏ, cho từng biến thể một đơn vị.
    pub(crate) async fn automatic_prices(
        &self,
        tenant: &TenantContext,
        org_id: OrgId,
        at: Timestamp,
        prices: &[(Uuid, Money)],
    ) -> Result<Vec<AutomaticPrice>, Error> {
        let Some((_, first)) = prices.first() else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for chunk in prices.chunks(crate::api::MAX_CART_LINES) {
            let mut cart = Cart::new(org_id, first.currency(), at);
            cart.lines = chunk
                .iter()
                .map(|(variant, price)| CartLine::new(variant.to_string(), *variant, 1, *price))
                .collect();
            let options = Options { unconditional_only: true, ..Options::default() };
            let outcome = self.evaluate_cart(tenant, &cart, &options).await?;
            for (variant, price) in chunk {
                let key = variant.to_string();
                let discounts: Vec<_> =
                    outcome.evaluation.lines.iter().filter(|line| line.line_key == key).collect();
                let total: i64 = discounts.iter().map(|line| line.amount.minor()).sum();
                let Some(lead) = discounts.first() else { continue };
                let ends_at = outcome
                    .applied
                    .iter()
                    .filter(|(program, _)| {
                        discounts.iter().any(|line| line.promotion_id == program.id)
                    })
                    .map(|(program, _)| program.ends_at)
                    .min()
                    .unwrap_or(at);
                out.push(AutomaticPrice {
                    variant_id: *variant,
                    promotion_id: lead.promotion_id,
                    sale_price: Money::new(price.minor() - total, price.currency()),
                    ends_at,
                });
            }
        }
        Ok(out)
    }
}

#[async_trait::async_trait]
impl Promotions for Provider {
    async fn evaluate(&self, tenant: &TenantContext, cart: &Cart) -> Result<Evaluation, Error> {
        Ok(self.0.evaluate_cart(tenant, cart, &Options::default()).await?.evaluation)
    }

    async fn eligible(&self, tenant: &TenantContext, cart: &Cart) -> Result<Eligibility, Error> {
        self.0.eligible(tenant, cart).await
    }

    async fn reserve(
        &self,
        tenant: &TenantContext,
        request: &ReserveRequest,
    ) -> Result<Submitted<Redemption>, Error> {
        self.0.reserve(tenant, request).await
    }

    async fn confirm(&self, tenant: &TenantContext, order_id: Uuid) -> Result<Redemption, Error> {
        self.0.confirm(tenant, order_id).await
    }

    async fn release(
        &self,
        tenant: &TenantContext,
        order_id: Uuid,
    ) -> Result<Option<Redemption>, Error> {
        self.0.release(tenant, order_id, false).await
    }

    async fn redemption(
        &self,
        tenant: &TenantContext,
        order_id: Uuid,
    ) -> Result<Option<Redemption>, Error> {
        self.0
            .redemption_row(tenant, order_id)
            .await?
            .as_ref()
            .map(crate::redemption::present)
            .transpose()
    }

    async fn grant_voucher(
        &self,
        tenant: &TenantContext,
        request: &GrantVoucher,
    ) -> Result<Submitted<Voucher>, Error> {
        self.0.grant_voucher(tenant, request, Actor::system()).await
    }

    async fn vouchers_of(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        status: Option<VoucherStatus>,
    ) -> Result<Vec<Voucher>, Error> {
        self.0.vouchers_of(tenant, user_id, status).await
    }

    async fn automatic_prices(
        &self,
        tenant: &TenantContext,
        org_id: OrgId,
        at: Timestamp,
        prices: &[(Uuid, Money)],
    ) -> Result<Vec<AutomaticPrice>, Error> {
        self.0.automatic_prices(tenant, org_id, at, prices).await
    }
}

/// Nguồn giá sau giảm tự động cắm vào `catalog` (danh sách sản phẩm công khai): cùng phép tính với
/// [`Promotions::automatic_prices`](crate::api::Promotions::automatic_prices).
pub(crate) struct SalePrices(pub Arc<PromotionsService>);

#[async_trait::async_trait]
impl autoxide_catalog::api::SalePriceSource for SalePrices {
    async fn sale_prices(
        &self,
        tenant: &TenantContext,
        org_id: OrgId,
        at: Timestamp,
        prices: &[(Uuid, Money)],
    ) -> Result<Vec<autoxide_catalog::api::SalePrice>, Error> {
        let found = self.0.automatic_prices(tenant, org_id, at, prices).await?;
        Ok(found
            .into_iter()
            .map(|price| {
                autoxide_catalog::api::SalePrice::new(
                    price.variant_id,
                    price.sale_price,
                    price.ends_at,
                )
            })
            .collect())
    }
}

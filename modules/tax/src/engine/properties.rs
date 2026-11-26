//! Thử phá bộ tính thuế bằng property test.

use super::*;
use proptest::prelude::*;

/// Một thuế suất; `per_unit = false` thì không sinh thuế theo đơn vị.
fn rate(per_unit: bool) -> impl Strategy<Value = (Basis, i64, i64)> {
    prop_oneof![
        4 => (0i64..=10_000).prop_map(|bp| (Basis::Percent, bp, 0)),
        u32::from(per_unit) => (0i64..=5_000).prop_map(|unit| (Basis::PerUnit, 0, unit)),
        1 => Just((Basis::Exempt, 0, 0)),
    ]
}

/// Chuỗi thuế suất: mỗi tầng một ID riêng.
fn chain() -> impl Strategy<Value = Vec<ResolvedRate>> {
    chain_of(true)
}

fn chain_of(per_unit: bool) -> impl Strategy<Value = Vec<ResolvedRate>> {
    proptest::collection::vec(rate(per_unit), 0..3).prop_map(|rates| {
        rates
            .into_iter()
            .enumerate()
            .map(|(stage, (basis, bp, unit))| ResolvedRate {
                id: Uuid::from_u128(
                    u128::from(stage as u64) << 64
                        | u128::from(bp.unsigned_abs()) << 32
                        | u128::from(unit.unsigned_abs()),
                ),
                tax_kind: format!("kind{stage}"),
                basis,
                rate_percent: Decimal::new(bp, 2),
                per_unit: (basis == Basis::PerUnit).then(|| Money::new(unit, Currency::VND)),
                stage: i32::try_from(stage).unwrap(),
            })
            .collect()
    })
}

fn document() -> impl Strategy<Value = (PriceMode, Vec<(TaxableLine, Vec<ResolvedRate>)>)> {
    document_of(true)
}

fn document_of(
    per_unit: bool,
) -> impl Strategy<Value = (PriceMode, Vec<(TaxableLine, Vec<ResolvedRate>)>)> {
    let mode = prop_oneof![Just(PriceMode::Inclusive), Just(PriceMode::Exclusive)];
    let lines = (chain_of(per_unit), chain_of(per_unit)).prop_flat_map(|(first, second)| {
        proptest::collection::vec((0i64..=1_000_000_000_000, 0i64..=1_000, any::<bool>()), 1..8)
            .prop_map(move |lines| {
                lines
                    .into_iter()
                    .enumerate()
                    .map(|(index, (amount, quantity, pick))| {
                        let line = TaxableLine::new(
                            format!("l{index}"),
                            "c",
                            Money::new(amount, Currency::VND),
                        )
                        .with_quantity(Decimal::from(quantity));
                        (line, if pick { first.clone() } else { second.clone() })
                    })
                    .collect::<Vec<_>>()
            })
    });
    (mode, lines)
}

proptest! {
    #[test]
    fn tong_dong_khop_va_tong_chung_tu_khop(document in document()) {
        let (mode, lines) = document;
        let computed = match compute(mode, Currency::VND, &lines) {
            Ok(computed) => computed,
            // Chỉ một lỗi được phép: giá đã gồm thuế thấp hơn thuế theo đơn vị.
            Err(error) => {
                prop_assert!(mode == PriceMode::Inclusive && lines.iter().any(|(_, chain)| chain.iter().any(|rate| rate.basis == Basis::PerUnit)));
                prop_assert_eq!(error.fields()[0].code.as_str(), "range");
                return Ok(());
            }
        };
        prop_assert_eq!(computed.lines.len(), lines.len());
        for taxed in &computed.lines {
            prop_assert!(!taxed.net.is_negative(), "{:?}", taxed);
        }
        let mut by_kind: BTreeMap<(String, bool), i128> = BTreeMap::new();
        for (taxed, (line, chain)) in computed.lines.iter().zip(&lines) {
            prop_assert_eq!(&taxed.line_id, &line.line_id);
            let taxes: i128 = taxed.taxes.iter().map(|tax| i128::from(tax.amount.minor())).sum();
            prop_assert_eq!(i128::from(taxed.gross.minor()), i128::from(taxed.net.minor()) + taxes);
            match mode {
                PriceMode::Exclusive => prop_assert_eq!(taxed.net, line.amount),
                PriceMode::Inclusive => prop_assert_eq!(taxed.gross, line.amount),
            }
            prop_assert_eq!(taxed.taxes.len(), chain.len());
            for (tax, rate) in taxed.taxes.iter().zip(chain) {
                if rate.basis == Basis::Percent && rate.rate_percent.is_zero() {
                    prop_assert!(tax.amount.is_zero(), "thuế suất 0% phải ra 0: {:?}", taxed);
                }
                if tax.exempt {
                    prop_assert!(tax.amount.is_zero(), "thuế của dòng không chịu thuế: {:?}", taxed);
                }
                prop_assert!(!tax.amount.is_negative(), "thuế âm: {:?}", taxed);
                *by_kind.entry((tax.tax_kind.clone(), tax.exempt)).or_default() += i128::from(tax.amount.minor());
            }
        }
        let mut totals: BTreeMap<(String, bool), i128> = BTreeMap::new();
        for total in &computed.totals {
            *totals.entry((total.tax_kind.clone(), total.exempt)).or_default() += i128::from(total.amount.minor());
        }
        prop_assert_eq!(by_kind, totals);
    }

    #[test]
    fn gia_chua_thue_phan_tram_khong_am_va_thue_khong_am(
        // Chỉ thuế theo phần trăm: thuế theo đơn vị lớn hơn giá thì giá chưa thuế âm là đúng số học.
        document in document_of(false),
    ) {
        let (mode, lines) = document;
        let computed = compute(mode, Currency::VND, &lines).unwrap();
        for taxed in &computed.lines {
            prop_assert!(!taxed.net.is_negative(), "{:?}", taxed);
            for tax in &taxed.taxes {
                prop_assert!(!tax.amount.is_negative(), "{:?}", taxed);
            }
        }
    }

    #[test]
    fn so_tien_bien_khong_panic(
        amount in any::<i64>(),
        chain in chain(),
        mode in prop_oneof![Just(PriceMode::Inclusive), Just(PriceMode::Exclusive)],
        quantity in any::<i64>(),
    ) {
        let line = TaxableLine::new("l", "c", Money::new(amount, Currency::VND))
            .with_quantity(Decimal::from(quantity));
        let _ = compute(mode, Currency::VND, &[(line, chain)]);
    }

    #[test]
    fn so_luong_cuc_lon_khong_panic(
        lo in any::<u32>(),
        mid in any::<u32>(),
        hi in any::<u32>(),
        negative in any::<bool>(),
        scale in 0u32..=28,
        lines in 1usize..4,
        chain in chain(),
        mode in prop_oneof![Just(PriceMode::Inclusive), Just(PriceMode::Exclusive)],
    ) {
        // Số lượng do người gọi gửi (`DecimalString`, tới 28 chữ số): cộng hay nhân có thể vượt
        // giới hạn của `Decimal` — phải là lỗi, không phải panic.
        let quantity = Decimal::from_parts(lo, mid, hi, negative, scale);
        let document: Vec<_> = (0..lines)
            .map(|index| {
                let line = TaxableLine::new(format!("l{index}"), "c", Money::new(1_000, Currency::VND))
                    .with_quantity(quantity);
                (line, chain.clone())
            })
            .collect();
        let _ = compute(mode, Currency::VND, &document);
    }
}
fn rate_of(stage: i32, basis: Basis, percent: i64) -> ResolvedRate {
    ResolvedRate {
        id: Uuid::from_u128(u128::try_from(stage).unwrap() + 1),
        tax_kind: format!("kind{stage}"),
        basis,
        rate_percent: Decimal::new(percent, 2),
        per_unit: (basis == Basis::PerUnit).then(|| Money::new(49, Currency::VND)),
        stage,
    }
}

#[test]
fn tang_khong_chiu_thue_cuoi_chuoi_khong_nhan_phan_du_lam_tron() {
    // Ca tái hiện: giá đã gồm thuế, chuỗi [10.31%, không chịu thuế] — trước khi sửa, phần dư làm
    // tròn (-1 đồng) rơi vào tầng không chịu thuế.
    let chain = vec![rate_of(0, Basis::Percent, 1031), rate_of(1, Basis::Exempt, 0)];
    let lines: Vec<_> = [261_698_875_581, 340_453_309_228, 478_001_272_827]
        .into_iter()
        .enumerate()
        .map(|(index, amount)| {
            (
                TaxableLine::new(format!("l{index}"), "c", Money::new(amount, Currency::VND)),
                chain.clone(),
            )
        })
        .collect();
    let computed = compute(PriceMode::Inclusive, Currency::VND, &lines).unwrap();
    for (taxed, (line, _)) in computed.lines.iter().zip(&lines) {
        assert_eq!(taxed.gross, line.amount);
        assert!(taxed.taxes[1].exempt && taxed.taxes[1].amount.is_zero(), "{taxed:?}");
    }
}

#[test]
fn so_luong_qua_lon_la_loi_kiem_dau_vao_khong_panic() {
    // Ca tái hiện: số lượng 28 chữ số làm phép nhân `Decimal` tràn và panic trước khi sửa.
    let quantity = Decimal::from_parts(0, 0, 88_446_609, false, 0);
    let line =
        TaxableLine::new("l0", "c", Money::new(1_000, Currency::VND)).with_quantity(quantity);
    let error = compute(
        PriceMode::Inclusive,
        Currency::VND,
        &[(line, vec![rate_of(0, Basis::PerUnit, 0)])],
    )
    .unwrap_err();
    assert_eq!(error.code(), "validation_failed");
}

// ---- Bất biến làm tròn (D-20261126-tax-rounding-allocation) ----
//
// Bộ sinh ở trên ít khi chạm ca biên của làm tròn (số tiền lớn, tối đa hai tầng). Bộ này cố ý nhắm vào
// chỗ phần dư làm tròn hay rơi sai: số tiền nhỏ, chuỗi 3–4 tầng, thuế suất rất nhỏ hay 0%, thuế theo
// đơn vị với số lượng 0 hay số lẻ, nhiều dòng cùng nhóm.

/// Số tiền: nhỏ (phần làm tròn chiếm phần lớn) hoặc lớn.
fn edge_amount() -> impl Strategy<Value = i64> {
    prop_oneof![3 => 0i64..=1_000, 1 => 0i64..=1_000_000_000_000]
}

/// Số lượng: 0, số nguyên, hay số lẻ tới hai chữ số thập phân.
fn edge_quantity() -> impl Strategy<Value = Decimal> {
    prop_oneof![
        1 => Just(Decimal::ZERO),
        2 => (0i64..=1_000).prop_map(Decimal::from),
        2 => (0i64..=1_000, 1u32..=2).prop_map(|(value, scale)| Decimal::new(value, scale)),
    ]
}

fn edge_rate() -> impl Strategy<Value = (Basis, i64, i64)> {
    prop_oneof![
        1 => Just((Basis::Percent, 0, 0)),
        2 => (1i64..=10).prop_map(|bp| (Basis::Percent, bp, 0)),
        2 => prop::sample::select(vec![400i64, 500, 800, 1000, 1607, 6500]).prop_map(|bp| (Basis::Percent, bp, 0)),
        2 => (0i64..=10_000).prop_map(|bp| (Basis::Percent, bp, 0)),
        2 => (0i64..=60).prop_map(|unit| (Basis::PerUnit, 0, unit)),
        1 => Just((Basis::Exempt, 0, 0)),
    ]
}

fn edge_chain() -> impl Strategy<Value = Vec<ResolvedRate>> {
    proptest::collection::vec(edge_rate(), 0..=4).prop_map(|rates| {
        rates
            .into_iter()
            .enumerate()
            .map(|(stage, (basis, bp, unit))| ResolvedRate {
                id: Uuid::from_u128(
                    u128::from(stage as u64) << 64
                        | u128::from(bp.unsigned_abs()) << 32
                        | u128::from(unit.unsigned_abs()),
                ),
                tax_kind: format!("kind{stage}"),
                basis,
                rate_percent: Decimal::new(bp, 2),
                per_unit: (basis == Basis::PerUnit).then(|| Money::new(unit, Currency::VND)),
                stage: i32::try_from(stage).unwrap(),
            })
            .collect()
    })
}

type Document = (PriceMode, Vec<(TaxableLine, Vec<ResolvedRate>)>);

fn edge_document() -> impl Strategy<Value = Document> {
    let mode = prop_oneof![3 => Just(PriceMode::Inclusive), 1 => Just(PriceMode::Exclusive)];
    let lines = (edge_chain(), edge_chain()).prop_flat_map(|(first, second)| {
        proptest::collection::vec((edge_amount(), edge_quantity(), 0u8..4), 1..7).prop_map(
            move |lines| {
                lines
                    .into_iter()
                    .enumerate()
                    .map(|(index, (amount, quantity, pick))| {
                        let line = TaxableLine::new(
                            format!("l{index}"),
                            "c",
                            Money::new(amount, Currency::VND),
                        )
                        .with_quantity(quantity);
                        // Ba phần tư số dòng chung chuỗi đầu: nhóm nhiều dòng là ca chính.
                        (line, if pick < 3 { first.clone() } else { second.clone() })
                    })
                    .collect::<Vec<_>>()
            },
        )
    });
    (mode, lines)
}

/// Kiểm I1–I4 ở cấp dòng và cấp nhóm; trả lời mô tả chỗ sai đầu tiên.
pub(super) fn check_invariants(
    mode: PriceMode,
    lines: &[(TaxableLine, Vec<ResolvedRate>)],
) -> Result<(), String> {
    let computed = match compute(mode, Currency::VND, lines) {
        Ok(computed) => computed,
        Err(error) => {
            // Chỉ một lỗi được phép: giá đã gồm thuế thấp hơn thuế theo đơn vị nằm trong nó.
            let per_unit = lines
                .iter()
                .any(|(_, chain)| chain.iter().any(|rate| rate.basis == Basis::PerUnit));
            let range = error.fields().first().is_some_and(|field| field.code == "range");
            return if mode == PriceMode::Inclusive && per_unit && range {
                Ok(())
            } else {
                Err(format!("lỗi không mong đợi: {error:?}"))
            };
        }
    };
    // Tổng số lượng theo chuỗi (= nhóm): thuế theo đơn vị của nhóm là đơn giá × tổng này.
    let mut quantities: BTreeMap<Vec<Uuid>, Decimal> = BTreeMap::new();
    for (line, chain) in lines {
        *quantities.entry(chain.iter().map(|rate| rate.id).collect()).or_default() +=
            line.quantity.map_or(Decimal::ZERO, |quantity| quantity.0);
    }
    let mut by_column: BTreeMap<(String, String, bool), i128> = BTreeMap::new();
    for (taxed, (line, chain)) in computed.lines.iter().zip(lines) {
        let quantity = quantities[&chain.iter().map(|rate| rate.id).collect::<Vec<_>>()];
        // I4.
        if taxed.net.is_negative() {
            return Err(format!("I4 giá chưa thuế âm: {taxed:?}"));
        }
        // I3 ở dòng.
        let taxes: i128 = taxed.taxes.iter().map(|tax| i128::from(tax.amount.minor())).sum();
        if i128::from(taxed.gross.minor()) != i128::from(taxed.net.minor()) + taxes {
            return Err(format!("I3 dòng không cộng đúng: {taxed:?}"));
        }
        let kept = match mode {
            PriceMode::Exclusive => taxed.net == line.amount,
            PriceMode::Inclusive => taxed.gross == line.amount,
        };
        if !kept {
            return Err(format!("số tiền của dòng bị đổi: {taxed:?}"));
        }
        for (tax, rate) in taxed.taxes.iter().zip(chain) {
            // I1 ở dòng.
            if tax.amount.is_negative() {
                return Err(format!("I1 thuế âm ở dòng: {taxed:?}"));
            }
            // I2 ở dòng.
            let nominal_zero = match rate.basis {
                Basis::Exempt => true,
                Basis::Percent => rate.rate_percent.is_zero(),
                Basis::PerUnit => {
                    rate.per_unit.is_none_or(|unit| (unit.to_decimal() * quantity).is_zero())
                }
            };
            if nominal_zero && !tax.amount.is_zero() {
                return Err(format!("I2 thuế danh nghĩa 0 mà ra khác 0 ở dòng: {taxed:?}"));
            }
            *by_column
                .entry((tax.tax_kind.clone(), tax.rate_percent.clone(), tax.exempt))
                .or_default() += i128::from(tax.amount.minor());
        }
    }
    // I3 ở nhóm: tổng theo cột bằng tổng các dòng (kéo theo I1, I2 ở nhóm khi dòng đã đúng).
    let mut totals: BTreeMap<(String, String, bool), i128> = BTreeMap::new();
    for total in &computed.totals {
        if total.amount.is_negative() {
            return Err(format!("I1 thuế âm ở nhóm: {total:?}"));
        }
        *totals
            .entry((total.tax_kind.clone(), total.rate_percent.clone(), total.exempt))
            .or_default() += i128::from(total.amount.minor());
    }
    if by_column != totals {
        return Err(format!("I3 tổng nhóm khác tổng dòng: {by_column:?} ≠ {totals:?}"));
    }
    Ok(())
}

proptest! {
    #[test]
    fn bat_bien_lam_tron_o_dong_va_nhom(document in edge_document()) {
        let (mode, lines) = document;
        if let Err(message) = check_invariants(mode, &lines) {
            prop_assert!(false, "{}", message);
        }
    }
}

// ---- Ca tái hiện của các phản ví dụ (số kỳ vọng tính bằng mô hình phân số chính xác, độc lập) ----

fn stage(stage: i32, basis: Basis, percent: Decimal, unit: i64) -> ResolvedRate {
    ResolvedRate {
        id: Uuid::from_u128(u128::try_from(stage).unwrap() + 1),
        tax_kind: format!("kind{stage}"),
        basis,
        rate_percent: percent,
        per_unit: (basis == Basis::PerUnit).then(|| Money::new(unit, Currency::VND)),
        stage,
    }
}

fn pct(position: i32, percent: &str) -> ResolvedRate {
    stage(position, Basis::Percent, percent.parse().unwrap(), 0)
}

/// Tính giá đã gồm thuế, kiểm I1–I4, trả (giá chưa thuế từng dòng, thuế từng dòng theo tầng, tổng
/// thuế theo tầng).
fn inclusive(
    chain: &[ResolvedRate],
    amounts: &[i64],
    quantity: Decimal,
) -> (Vec<i64>, Vec<Vec<i64>>, Vec<i64>) {
    let lines: Vec<_> = amounts
        .iter()
        .enumerate()
        .map(|(index, amount)| {
            // Cả số lượng nằm ở dòng đầu: thuế theo đơn vị của nhóm = đơn giá × tổng số lượng.
            let quantity = if index == 0 { quantity } else { Decimal::ZERO };
            let line =
                TaxableLine::new(format!("l{index}"), "c", Money::new(*amount, Currency::VND))
                    .with_quantity(quantity);
            (line, chain.to_vec())
        })
        .collect();
    check_invariants(PriceMode::Inclusive, &lines).unwrap();
    let computed = compute(PriceMode::Inclusive, Currency::VND, &lines).unwrap();
    let nets = computed.lines.iter().map(|line| line.net.minor()).collect();
    let taxes = computed
        .lines
        .iter()
        .map(|line| line.taxes.iter().map(|tax| tax.amount.minor()).collect())
        .collect();
    let totals = (0..chain.len())
        .map(|position| computed.lines.iter().map(|line| line.taxes[position].amount.minor()).sum())
        .collect();
    (nets, taxes, totals)
}

#[test]
fn ba_tang_tang_cuoi_khong_nhan_phan_du_am() {
    // [65%, 10%, 10%], tổng 5: giá chưa thuế 3, TTĐB 2, tầng 10% thứ hai làm tròn ra 0. Cách cũ: tầng
    // cuối = 5 − 3 − 2 − 1 = −1.
    let chain = [pct(0, "65"), pct(1, "10"), pct(2, "10")];
    let (nets, _, totals) = inclusive(&chain, &[5], Decimal::ZERO);
    assert_eq!(nets, [3]);
    assert_eq!(totals, [2, 0, 0]);
}

#[test]
fn thue_suat_rat_nho_o_tang_cuoi_khong_am() {
    // [0,01%, 10%, 0,01%], tổng 5505: cách cũ [1, 501, −1]; phần thừa 1 trừ ở tầng 10%.
    let chain = [pct(0, "0.01"), pct(1, "10"), pct(2, "0.01")];
    let (nets, _, totals) = inclusive(&chain, &[5505], Decimal::ZERO);
    assert_eq!(nets, [5004]);
    assert_eq!(totals, [1, 500, 0]);
}

#[test]
fn thue_theo_don_vi_so_luong_le_cuoi_chuoi_khong_am() {
    // [0,01%, 16,07%, 1 đ/đơn vị] số lượng 0,5, tổng 5804: cách cũ [1, 804, −1].
    let chain = [pct(0, "0.01"), pct(1, "16.07"), stage(2, Basis::PerUnit, Decimal::ZERO, 1)];
    let (nets, _, totals) = inclusive(&chain, &[5804], Decimal::new(5, 1));
    assert_eq!(nets, [5000]);
    assert_eq!(totals, [1, 803, 0]);
}

#[test]
fn tang_thue_suat_khong_cuoi_chuoi_khong_nhan_phan_thieu() {
    // [4%, 4%, 0%], tổng 10: giá chưa thuế 9, hai tầng 4% làm tròn ra 0, thiếu 1. Không được dồn vào
    // tầng 0% — vào tầng 4% sau cùng.
    let chain = [pct(0, "4"), pct(1, "4"), pct(2, "0")];
    let (nets, _, totals) = inclusive(&chain, &[10], Decimal::ZERO);
    assert_eq!(nets, [9]);
    assert_eq!(totals, [0, 1, 0]);
}

#[test]
fn thue_theo_don_vi_so_luong_khong_cuoi_chuoi_khong_nhan_phan_thieu() {
    // [4%, 4%, 49 đ/đơn vị] số lượng 0: thuế theo đơn vị bằng 0, phần thiếu 1 vào tầng 4% sau cùng.
    let chain = [pct(0, "4"), pct(1, "4"), stage(2, Basis::PerUnit, Decimal::ZERO, 49)];
    let (nets, _, totals) = inclusive(&chain, &[10], Decimal::ZERO);
    assert_eq!(nets, [9]);
    assert_eq!(totals, [0, 1, 0]);
}

#[test]
fn chia_ve_dong_khong_ra_thue_am() {
    // [1%, 1%], hai dòng 26 + 26: nhóm 51 + [1, 0]. Cách cũ chia tầng 0 thành [1, 0] rồi lấy tầng 1 của
    // dòng = phần còn lại → [−1, +1]. Nay đơn vị thuế duy nhất về dòng có giá chưa thuế nhỏ hơn.
    let chain = [pct(0, "1"), pct(1, "1")];
    let (nets, taxes, totals) = inclusive(&chain, &[26, 26], Decimal::ZERO);
    assert_eq!(nets, [26, 25]);
    assert_eq!(taxes, [vec![0, 0], vec![1, 0]]);
    assert_eq!(totals, [1, 0]);
}

#[test]
fn chia_ve_dong_thue_theo_don_vi_so_tien_lon_khong_am() {
    // [10%, 2 đ/đơn vị] tổng số lượng 1, bốn dòng ~1e11: cách cũ cho dòng thứ ba −1 đ thuế theo đơn vị.
    let chain = [pct(0, "10"), stage(1, Basis::PerUnit, Decimal::ZERO, 2)];
    let amounts = [415_698_296_401, 846_558_977_934, 29_261_856_332, 626_942_279_638];
    let (nets, taxes, totals) = inclusive(&chain, &amounts, Decimal::ONE);
    assert_eq!(nets, [377_907_542_182, 769_599_070_848, 26_601_687_575, 569_947_526_943]);
    assert_eq!(
        taxes,
        [
            vec![37_790_754_219, 0],
            vec![76_959_907_085, 1],
            vec![2_660_168_757, 0],
            vec![56_994_752_694, 1],
        ]
    );
    assert_eq!(totals, [174_405_582_755, 2]);
}

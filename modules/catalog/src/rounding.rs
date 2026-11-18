//! Làm tròn giá đẹp (D-222): giá về bội số của `step` (vd 1.000 đồng), hay về dạng `k × step +
//! ending` (vd `step` 10.000, `ending` 9.000 → 129.000, 199.000). Chỉ dùng số nguyên trên đơn vị
//! nhỏ nhất, không số thực. Dùng cho bảng giá suy ra và đổi giá hàng loạt (và sau này giá theo kênh,
//! quy đổi tiền tệ).

/// Cách làm tròn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RoundingMode {
    Up,
    Down,
    Nearest,
}

impl RoundingMode {
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        match raw {
            "up" => Some(RoundingMode::Up),
            "down" => Some(RoundingMode::Down),
            "nearest" => Some(RoundingMode::Nearest),
            _ => None,
        }
    }
}

/// Giá đẹp gần `amount` theo `step` (> 0) và `ending` (0 ≤ ending < step). Không bao giờ âm:
/// làm tròn xuống dưới mức đầu tiên thì lấy mức đầu tiên ≥ 0.
pub(crate) fn round(amount: i64, step: i64, ending: i64, mode: RoundingMode) -> i64 {
    if step <= 0 || !(0..step).contains(&ending) {
        return amount;
    }
    let amount = i128::from(amount.max(0));
    let (step, ending) = (i128::from(step), i128::from(ending));
    // Mức thấp nhất ≤ amount có dạng k × step + ending.
    let below = (amount - ending).div_euclid(step) * step + ending;
    let above = if below == amount { below } else { below + step };
    let chosen = match mode {
        RoundingMode::Up => above,
        RoundingMode::Down => {
            if below < 0 {
                above
            } else {
                below
            }
        }
        RoundingMode::Nearest => {
            if below < 0 || above - amount <= amount - below {
                above
            } else {
                below
            }
        }
    };
    i64::try_from(chosen).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::{RoundingMode, round};

    #[test]
    fn lam_tron_duoi_9000() {
        let up = |amount| round(amount, 10_000, 9_000, RoundingMode::Up);
        assert_eq!(up(121_500), 129_000);
        assert_eq!(up(129_000), 129_000);
        assert_eq!(up(129_001), 139_000);
        assert_eq!(round(195_000, 10_000, 9_000, RoundingMode::Down), 189_000);
        assert_eq!(round(195_000, 10_000, 9_000, RoundingMode::Nearest), 199_000);
        assert_eq!(round(193_000, 10_000, 9_000, RoundingMode::Nearest), 189_000);
        assert_eq!(round(1_000, 10_000, 9_000, RoundingMode::Down), 9_000);
    }

    #[test]
    fn lam_tron_theo_buoc() {
        assert_eq!(round(12_345, 1_000, 0, RoundingMode::Nearest), 12_000);
        assert_eq!(round(12_500, 1_000, 0, RoundingMode::Nearest), 13_000);
        assert_eq!(round(12_001, 1_000, 0, RoundingMode::Up), 13_000);
        assert_eq!(round(12_999, 1_000, 0, RoundingMode::Down), 12_000);
        assert_eq!(round(-5, 1_000, 0, RoundingMode::Down), 0);
    }
}

//! Micro benchmarks for the kernel's hot paths (roadmap 71-G9): the small pure functions that run on nearly
//! every request — money arithmetic, rounding, permission checks, cursor encoding, slugs and email checks.
//! Run with `cargo bench -p autoxide-kernel --bench hot_paths`. Numbers are for comparing before and after a
//! change on the same machine; they are reported weekly by `.github/workflows/load.yml`, not used as a limit.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use autoxide_kernel::access::{Permission, Principal};
use autoxide_kernel::decimal::{self, Rounding};
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::pagination::text_key_cursor;
use autoxide_kernel::{email, slug};
use divan::black_box;
use uuid::Uuid;

fn main() {
    divan::main();
}

const READ: Permission = Permission::new("notes.read", "Read notes.");

#[divan::bench]
fn money_checked_add() -> Money {
    let a = Money::new(black_box(1_234_567), Currency::VND);
    a.checked_add(black_box(Money::new(7_654_321, Currency::VND))).unwrap()
}

#[divan::bench(args = [2, 3, 12])]
fn money_allocate(parts: u64) -> Vec<Money> {
    let ratios = vec![1u64; parts as usize];
    Money::new(black_box(10_000_007), Currency::VND).allocate(&ratios).unwrap()
}

#[divan::bench]
fn money_apply_rate_half_up() -> Money {
    let rate = decimal::parse("0.08").unwrap();
    Money::new(black_box(1_999_999), Currency::VND).apply_rate(rate, Rounding::HalfUp).unwrap()
}

#[divan::bench]
fn money_parse() -> Money {
    Money::parse(black_box("1234567"), "VND").unwrap()
}

#[divan::bench]
fn decimal_parse() -> impl Sized {
    decimal::parse(black_box("1234567.891234")).unwrap()
}

#[divan::bench]
fn permission_require(bencher: divan::Bencher) {
    let principal = Principal::new(
        Uuid::now_v7(),
        ["notes.read", "notes.write", "orders.read", "orders.write"].map(str::to_owned),
    );
    bencher.bench_local(|| black_box(&principal).require(&READ).unwrap());
}

#[divan::bench]
fn cursor_encode() -> String {
    text_key_cursor(black_box("name"), black_box("Nguyen Van A"))
}

#[divan::bench]
fn slug_from_title() -> String {
    slug::from_title(black_box("Cà phê sữa đá — bản đặc biệt 2026"), Uuid::nil())
}

#[divan::bench]
fn email_normalize_and_check() -> bool {
    let normalized = email::normalize(black_box("  Someone.Name@Example.COM "));
    email::is_valid(&normalized)
}

#[divan::bench]
fn uuid_v7() -> Uuid {
    Uuid::now_v7()
}

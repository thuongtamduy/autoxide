//! Login throttling: chặn đoán mật khẩu bằng cách thử liên tục.
//!
//! Đếm lần đăng nhập sai theo email (kể cả email không tồn tại — để không lộ email
//! nào có thật). Quá `max_failures` lần trong một `period` thì khoá đăng nhập của email
//! đó trong `period`, kể cả khi mật khẩu đúng. Đếm trong database nên mọi máy chủ dùng
//! chung một con số.
//!
//! Mỗi bước là MỘT câu `UPDATE` có điều kiện — database tự xếp hàng các request đồng
//! thời, không request nào ghi đè số đếm của request khác.

use std::time::Duration;

use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::sea_query::{Expr, ExprTrait};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set, SqlErr};
use uuid::Uuid;

use crate::token;

use self::entity::{Column, Entity};

/// Entity của bảng `auth_login_throttle` (SeaORM).
mod entity {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_login_throttle")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        #[sea_orm(unique)]
        pub account_key: String,
        pub failures: i32,
        pub window_started_at: DateTimeUtc,
        pub locked_until: Option<DateTimeUtc>,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) struct LoginThrottle {
    database: Database,
    max_failures: i32,
    period: chrono::Duration,
}

impl LoginThrottle {
    pub(crate) fn new(
        database: Database,
        max_failures: u32,
        period: Duration,
    ) -> Result<Self, Error> {
        let max_failures = i32::try_from(max_failures).map_err(Error::internal)?;
        let period = chrono::Duration::from_std(period).map_err(Error::internal)?;
        Ok(LoginThrottle { database, max_failures, period })
    }

    /// Lỗi 429 `auth.login_throttled` nếu đăng nhập của `email` đang bị khoá.
    pub(crate) async fn check(&self, email: &str) -> Result<(), Error> {
        let now = now();
        let row = Entity::find()
            .filter(Column::AccountKey.eq(key(email)))
            .one(self.database.write())
            .await?;
        match row.and_then(|row| row.locked_until) {
            Some(until) if until > now => {
                // Làm tròn lên: chờ đúng số giây báo thì chắc chắn hết khoá.
                let wait = std::cmp::max((until - now).num_milliseconds(), 0);
                let seconds = u64::try_from((wait + 999) / 1000).unwrap_or(1);
                Err(Error::too_many_requests(
                    "auth.login_throttled",
                    "Too many failed sign-in attempts for this email. Wait {seconds} seconds, \
                     then try again.",
                )
                .with_param("seconds", seconds)
                .with_retry_after(seconds))
            }
            _ => Ok(()),
        }
    }

    /// Xin một lượt thử cho `email`, TRƯỚC khi kiểm mật khẩu hay mã: đếm lượt này, rồi
    /// từ chối nếu email đang (hoặc vừa) bị khoá. Đếm trước rồi mới kiểm nên nhiều request
    /// bắn cùng lúc không vượt được số lần cho phép (D-079). Thử đúng thì gọi
    /// [`clear`](Self::clear).
    pub(crate) async fn reserve(&self, email: &str) -> Result<(), Error> {
        self.check(email).await?;
        self.record_failure(email).await?;
        self.check(email).await
    }

    /// Trả lại lượt vừa xin bằng [`reserve`](Self::reserve) khi lần thử đó không sai (vd
    /// mật khẩu đúng, còn chờ mã MFA): người dùng thật không bị tính oan.
    pub(crate) async fn refund(&self, email: &str) -> Result<(), Error> {
        Entity::update_many()
            .col_expr(Column::Failures, Expr::col(Column::Failures).sub(1))
            .filter(Column::AccountKey.eq(key(email)))
            .filter(Column::Failures.gt(0))
            .exec(self.database.write())
            .await?;
        Ok(())
    }

    /// Ghi một lần đăng nhập sai của `email`; đủ số lần thì khoá.
    pub(crate) async fn record_failure(&self, email: &str) -> Result<(), Error> {
        let key = key(email);
        let now = now();
        let window_start = now - self.period;
        // Vòng lặp chỉ chạy lại một lần, khi hai request cùng chèn dòng đầu tiên.
        for _ in 0..2 {
            // 1. Đang trong cửa sổ đếm: cộng một.
            let counted = Entity::update_many()
                .col_expr(Column::Failures, Expr::col(Column::Failures).add(1))
                .filter(Column::AccountKey.eq(key.as_str()))
                .filter(Column::WindowStartedAt.gt(window_start))
                .exec(self.database.write())
                .await?;
            if counted.rows_affected > 0 {
                return self.lock_if_exceeded(&key, now).await;
            }
            // 2. Cửa sổ cũ đã hết: bắt đầu cửa sổ mới với một lần sai.
            let restarted = Entity::update_many()
                .col_expr(Column::Failures, Expr::value(1))
                .col_expr(Column::WindowStartedAt, Expr::value(now))
                .filter(Column::AccountKey.eq(key.as_str()))
                .filter(Column::WindowStartedAt.lte(window_start))
                .exec(self.database.write())
                .await?;
            if restarted.rows_affected > 0 {
                return self.lock_if_exceeded(&key, now).await;
            }
            // 3. Chưa có dòng: chèn. Request khác vừa chèn trước thì quay lại bước 1.
            let inserted = entity::ActiveModel {
                id: Set(Uuid::now_v7()),
                account_key: Set(key.clone()),
                failures: Set(1),
                window_started_at: Set(now),
                locked_until: Set(None),
            }
            .insert(self.database.write())
            .await;
            match inserted {
                Ok(_) => return self.lock_if_exceeded(&key, now).await,
                Err(error)
                    if matches!(error.sql_err(), Some(SqlErr::UniqueConstraintViolation(_))) => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    /// Đăng nhập đúng: quên các lần sai trước đó.
    pub(crate) async fn clear(&self, email: &str) -> Result<(), Error> {
        Entity::delete_many()
            .filter(Column::AccountKey.eq(key(email)))
            .exec(self.database.write())
            .await?;
        Ok(())
    }

    /// Vượt ngưỡng thì khoá tới `now + period` và bắt đầu đếm lại từ đầu, để hết khoá
    /// là có đủ số lần thử như mới.
    async fn lock_if_exceeded(&self, key: &str, now: DateTime<Utc>) -> Result<(), Error> {
        let locked = Entity::update_many()
            .col_expr(Column::LockedUntil, Expr::value(Some(now + self.period)))
            .col_expr(Column::Failures, Expr::value(0))
            .col_expr(Column::WindowStartedAt, Expr::value(now))
            .filter(Column::AccountKey.eq(key))
            // Lượt vượt mức mới khoá: `max_failures` lượt đầu vẫn được kiểm.
            .filter(Column::Failures.gt(self.max_failures))
            .exec(self.database.write())
            .await?;
        if locked.rows_affected > 0 {
            tracing::warn!("sign-in locked after too many failed attempts");
            metrics::counter!("auth_login_locks_total").increment(1);
        }
        Ok(())
    }
}

/// Khoá đếm của một email: SHA-256 của email đã chuẩn hoá.
fn key(email: &str) -> String {
    token::sha256(&email.trim().to_lowercase())
}

fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use autoxide_kernel::error::ErrorKind;
    use autoxide_testkit::TestDatabase;

    async fn throttle(test: &TestDatabase, max_failures: u32) -> LoginThrottle {
        let database = crate::testing::database(test).await;
        LoginThrottle::new(database, max_failures, Duration::from_secs(15 * 60)).unwrap()
    }

    async fn row(throttle: &LoginThrottle, email: &str) -> Option<entity::Model> {
        Entity::find()
            .filter(Column::AccountKey.eq(key(email)))
            .one(throttle.database.write())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn sai_qua_so_lan_thi_khoa_roi_dung_thi_quen() {
        TestDatabase::run(async |test| {
            let throttle = throttle(test, 3).await;

            // Ba lượt đầu được kiểm (dù sai); lượt thứ tư bị khoá.
            for _ in 0..3 {
                throttle.reserve("Ada@Example.com").await.unwrap();
            }
            let error = throttle.reserve(" ADA@example.com").await.unwrap_err();
            assert_eq!(error.kind(), ErrorKind::TooManyRequests);
            assert_eq!(error.code(), "auth.login_throttled");
            let wait = error.retry_after().unwrap();
            assert!((899..=900).contains(&wait), "{wait}");
            // Email khác không bị ảnh hưởng.
            throttle.check("bob@example.com").await.unwrap();

            throttle.clear("ada@example.com").await.unwrap();
            throttle.check("ada@example.com").await.unwrap();
            assert!(row(&throttle, "ada@example.com").await.is_none());
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn het_khoa_va_het_cua_so_thi_dem_lai() {
        TestDatabase::run(async |test| {
            let throttle = throttle(test, 2).await;
            for _ in 0..3 {
                let _ = throttle.reserve("ada@example.com").await;
            }
            assert!(throttle.check("ada@example.com").await.is_err());

            // Cho thời gian trôi qua: khoá và cửa sổ đều đã hết.
            let past = now() - chrono::Duration::minutes(16);
            Entity::update_many()
                .col_expr(Column::LockedUntil, Expr::value(Some(past)))
                .col_expr(Column::WindowStartedAt, Expr::value(past))
                .exec(throttle.database.write())
                .await
                .unwrap();
            throttle.check("ada@example.com").await.unwrap();

            throttle.record_failure("ada@example.com").await.unwrap();

            let counted = row(&throttle, "ada@example.com").await.unwrap();
            assert_eq!(counted.failures, 1);
            throttle.check("ada@example.com").await.unwrap();
        })
        .await
        .unwrap();
    }

    /// Nhiều lần sai cùng lúc cho cùng một email (kể cả lần đầu tiên, lúc chưa có dòng
    /// nào): không lần nào bị mất — trên cả hai database.
    #[tokio::test]
    async fn sai_dong_thoi_dem_du_tung_lan() {
        TestDatabase::run(async |test| {
            let throttle = Arc::new(throttle(test, 100).await);

            let attempts: Vec<_> = (0..8)
                .map(|_| {
                    let throttle = throttle.clone();
                    tokio::spawn(async move { throttle.record_failure("ada@example.com").await })
                })
                .collect();
            for attempt in attempts {
                attempt.await.unwrap().unwrap();
            }

            assert_eq!(row(&throttle, "ada@example.com").await.unwrap().failures, 8);
        })
        .await
        .unwrap();
    }

    /// Nhiều lần thử bắn cùng lúc: không bao giờ quá `max_failures` lượt được kiểm, và
    /// email bị khoá sau đó — trên cả hai database (D-079).
    #[tokio::test]
    async fn thu_dong_thoi_khong_vuot_so_lan_cho_phep() {
        TestDatabase::run(async |test| {
            let throttle = Arc::new(throttle(test, 5).await);

            let attempts: Vec<_> = (0..40)
                .map(|_| {
                    let throttle = throttle.clone();
                    tokio::spawn(async move { throttle.reserve("ada@example.com").await })
                })
                .collect();
            let mut allowed = 0;
            for attempt in attempts {
                if attempt.await.unwrap().is_ok() {
                    allowed += 1;
                }
            }

            assert!(allowed <= 5, "{allowed} attempts got through");
            assert!(throttle.check("ada@example.com").await.is_err());
        })
        .await
        .unwrap();
    }
}

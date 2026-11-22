//! Khoá dòng và cập nhật có điều kiện — một cách cho mọi module khi nhiều request cùng sửa một
//! dòng (D-181).
//!
//! Hai công cụ, chạy như nhau trên SQLite và Postgres:
//!
//! - [`lock`] / [`lock_one`]: đọc các dòng và **khoá chúng tới hết transaction ghi**. Postgres:
//!   `SELECT … FOR UPDATE` — transaction khác muốn khoá hay sửa cùng dòng phải chờ transaction
//!   này xong, rồi thấy giá trị mới. SQLite: câu `SELECT` thường là đủ, vì mọi transaction ghi
//!   mở bằng [`Database::begin_write`](crate::database::Database::begin_write) (`BEGIN
//!   IMMEDIATE`) giữ quyền ghi cả database từ đầu tới cuối — không ai chen vào được.
//!   Dùng khi phải đọc, xét rồi mới ghi (kiểm vòng lặp trong cây, kiểm quyền còn hiệu lực…).
//! - [`adjust`]: **cộng / trừ có điều kiện trong một câu** — `UPDATE … SET c = c + Δ WHERE …`
//!   kèm điều kiện kết quả nằm trong giới hạn — rồi đếm số dòng bị đổi. Đây là cách chuẩn để trừ số dư: không đọc
//!   trước, nên không có khoảng hở giữa lúc đọc và lúc ghi; số dư không bao giờ xuống dưới mức
//!   sàn, và không bao giờ tràn `i64` (SQLite đổi số nguyên tràn thành số thực một cách âm thầm;
//!   Postgres báo lỗi — chặn trước bằng điều kiện thì hai bên như nhau).
//!
//! - [`lock_key`]: khoá theo tên tới hết transaction ghi, khi không có dòng ổn định để khoá.
//!
//! Thứ tự khoá chống deadlock: khi một transaction khoá nhiều dòng, luôn khoá theo cùng một thứ
//! tự (vd ID tăng dần — [`lock`] làm vậy khi được giao cột sắp xếp), và khoá trước khi ghi.
//! Hai transaction khoá A rồi B và B rồi A sẽ chờ nhau mãi; Postgres phát hiện và huỷ một bên
//! với lỗi `40P01` (xem [`Database::write_with_retry`](crate::database::Database::write_with_retry)).

use sea_orm::sea_query::{Expr, ExprTrait as _, IntoCondition};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DbBackend, DbErr, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, Select,
};
use sha2::{Digest as _, Sha256};

/// Đọc và khoá mọi dòng `select` chọn, tới hết `transaction`. `order` là cột sắp xếp khi khoá
/// (thường là khoá chính): nhiều transaction khoá cùng nhóm dòng theo cùng thứ tự thì không
/// deadlock. `transaction` phải là transaction GHI (`Database::begin_write`): trên Postgres
/// transaction chỉ đọc từ chối `FOR UPDATE`, và ngoài transaction thì khoá nhả ngay sau câu lệnh
/// (nhận kiểu `ConnectionTrait` chung để hàm dùng chung được cả `&DatabaseTransaction` lẫn
/// `&C` của hàm gọi nó).
pub async fn lock<E: EntityTrait, C: ConnectionTrait>(
    transaction: &C,
    select: Select<E>,
    order: E::Column,
) -> Result<Vec<E::Model>, DbErr> {
    // `lock_exclusive` sinh `FOR UPDATE` trên Postgres và không sinh gì trên SQLite (thư viện
    // bỏ qua mệnh đề khoá ở SQLite) — xem tài liệu đầu file vì sao vậy là đủ.
    select.order_by_asc(order).lock_exclusive().all(transaction).await
}

/// Đọc và khoá dòng `select` chọn (tối đa một dòng, vd `Entity::find_by_id(id)` kèm điều kiện),
/// tới hết `transaction`. `None`: không có dòng nào khớp — kể cả khi dòng vừa bị transaction
/// khác đổi để không còn khớp (Postgres chờ transaction đó xong rồi xét lại điều kiện).
pub async fn lock_one<E: EntityTrait, C: ConnectionTrait>(
    transaction: &C,
    select: Select<E>,
) -> Result<Option<E::Model>, DbErr> {
    select.lock_exclusive().one(transaction).await
}

/// Đọc dòng `select` chọn (tối đa một dòng) và giữ **khoá chia sẻ** tới hết `transaction` (bước 36):
/// nhiều transaction cùng giữ khoá chia sẻ trên một dòng được, nhưng bên muốn [`lock_one`] (khoá độc
/// quyền) hay sửa dòng đó phải chờ chúng xong. Vd mọi bút toán giữ khoá chia sẻ trên dòng kỳ kế toán,
/// còn khoá sổ khoá độc quyền: khoá sổ chờ các bút toán đang ghi, bút toán mới chờ khoá sổ xong rồi
/// thấy kỳ đã khoá. Postgres: `SELECT … FOR SHARE`. SQLite: câu `SELECT` thường (mọi transaction ghi
/// đã giữ quyền ghi cả database — xem đầu file).
pub async fn lock_one_shared<E: EntityTrait, C: ConnectionTrait>(
    transaction: &C,
    select: Select<E>,
) -> Result<Option<E::Model>, DbErr> {
    select.lock_shared().one(transaction).await
}

/// Khoá **theo tên** tới hết `transaction` ghi: hai transaction khoá cùng `key` thì bên sau chờ bên
/// trước commit / rollback. Dùng khi việc cần tuần tự hoá không gắn với một dòng có sẵn — vd "mọi
/// lần tính lại của (khách, ngôn ngữ)" khi chưa có dòng nào, hay khi các dòng bị xoá / chèn lại
/// (đổi ID) giữa chừng nên khoá dòng không còn chắc trúng. Sau khi có khoá, đọc lại mọi thứ cần
/// xét: câu đọc mới thấy những gì transaction trước đã commit.
///
/// Postgres: `pg_advisory_xact_lock` với số 64 bit lấy từ SHA-256 của `key` (trùng số giữa hai tên
/// khác nhau chỉ làm chờ thừa, không sai). SQLite: không làm gì — transaction ghi mở bằng
/// [`Database::begin_write`](crate::database::Database::begin_write) (`BEGIN IMMEDIATE`) đã giữ
/// quyền ghi cả database. `transaction` phải là transaction GHI (ngoài transaction, khoá Postgres
/// nhả ngay sau câu lệnh). Một transaction giữ nhiều khoá thì luôn lấy theo cùng thứ tự (vd tên
/// tăng dần) để không deadlock. Đặt tên có tiền tố module (`catalog.similar:<…>`).
pub async fn lock_key<C: ConnectionTrait>(transaction: &C, key: &str) -> Result<(), DbErr> {
    let backend = transaction.get_database_backend();
    if backend != DbBackend::Postgres {
        return Ok(());
    }
    transaction
        .execute_raw(crate::database::statement(
            backend,
            "SELECT pg_advisory_xact_lock($1)",
            [key_number(key).into()],
        ))
        .await?;
    Ok(())
}

/// Số khoá advisory của `key`: 8 byte đầu của SHA-256 — như nhau trên mọi máy và mọi phiên bản.
fn key_number(key: &str) -> i64 {
    let digest = Sha256::digest(key.as_bytes());
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    i64::from_be_bytes(bytes)
}

/// Giới hạn cho [`adjust`]: giá trị của cột SAU khi cộng phải nằm trong `[floor, ceiling]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    pub floor: i64,
    pub ceiling: i64,
}

impl Bounds {
    /// Không âm: `[0, i64::MAX]` — số dư không được xuống dưới 0.
    pub const NON_NEGATIVE: Bounds = Bounds { floor: 0, ceiling: i64::MAX };
    /// Chỉ chặn tràn: `[i64::MIN, i64::MAX]` (vd số dư được phép âm tới hạn mức riêng — khi đó
    /// đặt `floor` là hạn mức).
    pub const ANY: Bounds = Bounds { floor: i64::MIN, ceiling: i64::MAX };
}

/// Cộng `delta` (âm là trừ) vào cột số nguyên `column` của các dòng khớp `filter`, chỉ ở dòng
/// mà kết quả nằm trong `bounds`. Trả số dòng đã đổi — với một dòng theo ID: `1` là đã trừ /
/// cộng, `0` là không có dòng đó hoặc không đủ (service tự phân biệt nếu cần, vd đọc lại).
///
/// Một câu `UPDATE`, không đọc trước: hai request trừ cùng lúc trên Postgres nối đuôi nhau ở
/// dòng đó (request sau xét lại điều kiện trên giá trị mới), trên SQLite vốn một bên ghi.
///
/// ```ignore
/// // Trừ 500 khỏi số dư của ví `id`, không cho âm.
/// let changed = rows::adjust::<wallet::Entity, _>(
///     &transaction, wallet::Column::BalanceMinor, wallet::Column::Id.eq(id), -500,
///     Bounds::NON_NEGATIVE,
/// ).await?;
/// if changed == 0 { return Err(insufficient_funds()); }
/// ```
pub async fn adjust<E, C>(
    connection: &C,
    column: E::Column,
    filter: impl IntoCondition,
    delta: i64,
    bounds: Bounds,
) -> Result<u64, DbErr>
where
    E: EntityTrait,
    C: ConnectionTrait,
{
    // Điều kiện `floor <= c + Δ <= ceiling` viết lại thành `floor − Δ <= c <= ceiling − Δ`, để
    // database không phải tính `c + Δ` (có thể tràn) khi xét điều kiện. Tính giới hạn bằng
    // `i128` rồi bỏ vế nào vượt khỏi `i64` — vế đó luôn đúng với mọi giá trị `i64` của cột.
    let low = i128::from(bounds.floor) - i128::from(delta);
    let high = i128::from(bounds.ceiling) - i128::from(delta);
    if low > high || low > i128::from(i64::MAX) || high < i128::from(i64::MIN) {
        return Ok(0);
    }
    let mut update = E::update_many()
        .col_expr(column, Expr::col(column).add(delta))
        .filter(filter.into_condition());
    if let Ok(low) = i64::try_from(low) {
        update = update.filter(column.gte(low));
    }
    if let Ok(high) = i64::try_from(high) {
        update = update.filter(column.lte(high));
    }
    Ok(update.exec(connection).await?.rows_affected)
}

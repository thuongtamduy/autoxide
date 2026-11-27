// Một lượt migrate placement một lúc, trên MỌI đường: lệnh `migrate up --all`, job của API, máy chủ lúc khởi động
// (D-20261126-placement-migration-exclusive). Hai lớp:
//
// 1. `MigrationLock` (khoá của tiến trình: advisory lock của Postgres, khoá file của SQLite) — loại trừ thật sự,
//    tự nhả khi tiến trình chết.
// 2. Dòng `active` duy nhất của `kernel_migration_runs` — để người vận hành thấy lượt đang chạy (cả lượt chạy từ
//    dòng lệnh) và để API báo 409 thay vì xếp lượt thứ hai.
//
// Hàm ở đây chỉ gọi khi ĐANG giữ khoá. Vì người giữ dòng `running` luôn giữ khoá, ai cầm được khoá mà thấy dòng
// `running` của người khác thì người đó đã chết giữa chừng: đánh dấu `failed` (phần đã áp dụng vẫn còn, chạy lại
// sẽ bỏ qua). Dòng `pending` là job đã xếp mà chưa chạy: để yên, trừ khi quá [`STALE_PENDING_SECONDS`] (job đã mất).

use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use uuid::Uuid;

use super::entity::{self, Column, Entity};
use super::{ACTIVE, PlacementMigrationList, now};
use crate::database::Database;
use crate::error::Error;

// Lượt `pending` lâu hơn chừng này mà chưa chạy: job của nó đã mất (hết lượt thử, bị xoá).
const STALE_PENDING_SECONDS: i64 = 60 * 60;

// Lý do ghi vào lượt mà tiến trình chạy nó đã dừng.
const INTERRUPTED: &str = "interrupted: the process running it stopped before it finished; what was \
                           already applied is kept, and the next run continues from there";
const NEVER_STARTED: &str = "never started: its job is gone; start a new run";

// Lượt nào cần giữ chỗ.
pub(crate) enum Claim<'a> {
    // Lượt đã xếp qua API (job chạy nó).
    Queued(Uuid),
    // Lượt mới chạy ngay (lệnh, `Placements::up_all`): `placement` như cột cùng tên.
    Direct { placement: Option<&'a str> },
}

// Dọn lượt của tiến trình đã chết, rồi giữ chỗ `active` cho lượt `claim`. Trả id của lượt đang giữ; `None`
// khi lượt `Queued` đã xong (job chạy lại sau khi lượt đã kết thúc: không còn gì làm). Lượt `Direct` gặp một lượt
// `pending` cùng phạm vi (hay chính nó chạy mọi placement) thì nhận luôn lượt đó — job của nó sau này thấy lượt đã
// xong và thôi; khác phạm vi thì `kernel.migration_run_busy`.
pub(crate) async fn claim(control: &Database, claim: Claim<'_>) -> Result<Option<Uuid>, Error> {
    let keep = match claim {
        Claim::Queued(id) => Some(id),
        Claim::Direct { .. } => None,
    };
    let active = recover(control, keep).await?;
    match (claim, active) {
        (Claim::Queued(id), Some(row)) if row.id == id => {
            Ok(Some(running(control, id, None).await?))
        }
        (Claim::Queued(id), None) => {
            let Some(row) = Entity::find_by_id(id).one(control.write()).await? else {
                return Ok(None);
            };
            if !matches!(row.status.as_str(), "pending" | "running") {
                return Ok(None);
            }
            Ok(Some(running(control, id, None).await?))
        }
        // Lượt khác đang giữ chỗ: lượt của job này đã xong (bị lượt dòng lệnh nhận rồi khép) thì thôi; còn chờ thì
        // job thử lại sau.
        (Claim::Queued(id), Some(row)) => {
            let own = Entity::find_by_id(id).one(control.write()).await?;
            match own {
                Some(own) if matches!(own.status.as_str(), "pending" | "running") => {
                    Err(busy(row.id))
                }
                _ => Ok(None),
            }
        }
        (Claim::Direct { placement }, Some(row))
            if placement.is_none() || row.placement.as_deref() == placement =>
        {
            let scope = placement.map(str::to_owned);
            Ok(Some(running(control, row.id, Some(scope)).await?))
        }
        (Claim::Direct { .. }, Some(row)) => Err(busy(row.id)),
        (Claim::Direct { placement }, None) => insert_direct(control, placement).await,
    }
}

// Lượt `Direct` mới. Va khoá unique `active_key` (API vừa xếp một lượt giữa lúc đọc và lúc chèn): nhận lại theo luật
// trên — một lần là đủ, vì API chỉ chèn được khi chưa có lượt `active`.
async fn insert_direct(control: &Database, placement: Option<&str>) -> Result<Option<Uuid>, Error> {
    match try_insert_direct(control, placement).await {
        Err(error) if error.code() == UNIQUE_CLASH => {
            let active =
                Entity::find().filter(Column::ActiveKey.eq(ACTIVE)).one(control.write()).await?;
            match active {
                Some(row) if placement.is_none() || row.placement.as_deref() == placement => {
                    let scope = placement.map(str::to_owned);
                    Ok(Some(running(control, row.id, Some(scope)).await?))
                }
                Some(row) => Err(busy(row.id)),
                None => try_insert_direct(control, placement).await,
            }
        }
        other => other,
    }
}

const UNIQUE_CLASH: &str = "kernel.migration_run_busy";

async fn try_insert_direct(
    control: &Database,
    placement: Option<&str>,
) -> Result<Option<Uuid>, Error> {
    let at = now();
    let row = entity::ActiveModel {
        id: Set(Uuid::now_v7()),
        placement: Set(placement.map(str::to_owned)),
        status: Set("running".to_owned()),
        requested_by: Set(None),
        result: Set("[]".to_owned()),
        error: Set(None),
        created_at: Set(at),
        updated_at: Set(at),
        active_key: Set(Some(ACTIVE.to_owned())),
    }
    .insert(control.write())
    .await
    .map_err(|error| {
        if matches!(error.sql_err(), Some(sea_orm::SqlErr::UniqueConstraintViolation(_))) {
            busy(Uuid::nil())
        } else {
            Error::from(error)
        }
    })?;
    Ok(Some(row.id))
}

// Đang giữ khoá: kết thúc lượt `active` bỏ dở (trừ lượt `keep`), trả lượt `active` còn lại.
pub(crate) async fn recover(
    control: &Database,
    keep: Option<Uuid>,
) -> Result<Option<entity::Model>, Error> {
    let active = Entity::find().filter(Column::ActiveKey.eq(ACTIVE)).one(control.write()).await?;
    match active {
        Some(row) if Some(row.id) != keep && stale(&row) => {
            let reason = if row.status == "running" { INTERRUPTED } else { NEVER_STARTED };
            tracing::warn!(run_id = %row.id, status = %row.status, "placement migration run left unfinished");
            finish(control, row.id, None, Some(reason.to_owned())).await?;
            Ok(None)
        }
        other => Ok(other),
    }
}

// Đang giữ khoá: lượt `running` của người khác là của tiến trình đã chết; lượt `pending` quá cũ là job đã mất.
fn stale(row: &entity::Model) -> bool {
    match row.status.as_str() {
        "running" => true,
        "pending" => now().duration_since(row.updated_at).as_secs() > STALE_PENDING_SECONDS,
        _ => false,
    }
}

// Đặt lượt `id` thành `running` và giữ khoá `active`; `scope`: đổi phạm vi (lượt `Direct` nhận lượt đang chờ).
async fn running(
    control: &Database,
    id: Uuid,
    scope: Option<Option<String>>,
) -> Result<Uuid, Error> {
    let mut active = entity::ActiveModel {
        id: Set(id),
        status: Set("running".to_owned()),
        updated_at: Set(now()),
        active_key: Set(Some(ACTIVE.to_owned())),
        ..Default::default()
    };
    if let Some(scope) = scope {
        active.placement = Set(scope);
    }
    active.update(control.write()).await?;
    Ok(id)
}

// Kết thúc lượt `id`: `done` khi không có lỗi, `failed` khi có; nhả khoá `active` cho lượt sau.
pub(crate) async fn finish(
    control: &Database,
    id: Uuid,
    applied: Option<&[PlacementMigrationList]>,
    error: Option<String>,
) -> Result<(), Error> {
    let status = if error.is_some() { "failed" } else { "done" };
    let mut active = entity::ActiveModel {
        id: Set(id),
        status: Set(status.to_owned()),
        updated_at: Set(now()),
        active_key: Set(None),
        error: Set(error),
        ..Default::default()
    };
    if let Some(applied) = applied {
        active.result = Set(serde_json::to_string(applied).map_err(Error::internal)?);
    }
    active.update(control.write()).await?;
    tracing::info!(run_id = %id, status, "placement migration run finished");
    Ok(())
}

fn busy(run: Uuid) -> Error {
    Error::conflict(
        "kernel.migration_run_busy",
        "A migration run is already in progress (`{run}`); wait for it to finish.",
    )
    .with_param("run", run)
}

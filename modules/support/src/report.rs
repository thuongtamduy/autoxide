//! Báo cáo hỗ trợ (bước 53 mục 3): số phiếu, thời gian phản hồi / giải quyết trung vị, tỉ lệ quá
//! hạn, mức hài lòng — theo phạm vi người gọi, nút và khoảng thời gian tạo phiếu. Đọc cột cần thiết
//! của các phiếu trong khoảng rồi tính trong Rust (trung vị không viết chung được cho hai
//! database).

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::tenancy::{OrgId, OrgTree, ScopeExt, TenantContext};
use chrono::{DateTime, Utc};
use sea_orm::sea_query::Condition;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::entity::ticket;
use crate::permissions::TICKETS_READ;
use crate::service::SupportService;

/// Cột một phiếu báo cáo cần: lúc tạo, lúc trả lời đầu, lúc xử lý xong, quá hạn, mức hài lòng.
type Row = (DateTime<Utc>, Option<DateTime<Utc>>, Option<DateTime<Utc>>, bool, Option<i16>);

/// Số phiếu tối đa một báo cáo đọc (khoảng dài hơn thì thu hẹp khoảng).
const MAX_ROWS: u64 = 100_000;

/// Support figures for tickets created in a period.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[schema(as = SupportReport)]
pub(crate) struct Summary {
    /// Tickets created in the period.
    pub tickets: u64,
    /// Of those, solved or closed.
    pub solved: u64,
    /// Of those, that missed a response-time target.
    pub breached: u64,
    /// `breached` per 10000 tickets.
    pub breach_rate_bp: u64,
    /// Median minutes from opening to the first answer (wall clock), of answered tickets.
    pub median_first_response_minutes: Option<i64>,
    /// Median minutes from opening to solved (wall clock), of solved tickets.
    pub median_resolve_minutes: Option<i64>,
    /// Ratings given (1 to 5).
    pub satisfaction_count: u64,
    /// Average rating × 100 (e.g. 450 = 4.5).
    pub satisfaction_avg_x100: Option<i64>,
}

impl SupportService {
    /// Báo cáo các phiếu tạo trong `[from, to)` trong phạm vi `support.tickets.read` của người gọi;
    /// `org`: chỉ nút đó và nhánh dưới.
    pub(crate) async fn summary(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: Option<Uuid>,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Summary, Error> {
        if to <= from {
            return Err(Error::validation(vec![FieldError::new(
                "to",
                "invalid",
                "must be after `from`",
            )]));
        }
        let scope = principal.scope_condition(
            &TICKETS_READ,
            ticket::Column::OrgId,
            Some(ticket::Column::AssigneeUserId),
        )?;
        let mut condition = Condition::all()
            .add(scope)
            .add(ticket::Column::CreatedAt.gte(from))
            .add(ticket::Column::CreatedAt.lt(to));
        if let Some(org) = org {
            condition = condition.add(OrgTree::subtree_condition(
                tenant.id(),
                &[OrgId::from_uuid(org)],
                ticket::Column::OrgId,
            ));
        }
        type Row = (DateTime<Utc>, Option<DateTime<Utc>>, Option<DateTime<Utc>>, bool, Option<i16>);
        let rows: Vec<Row> = ticket::Entity::find()
            .select_only()
            .column(ticket::Column::CreatedAt)
            .column(ticket::Column::FirstRespondedAt)
            .column(ticket::Column::SolvedAt)
            .column(ticket::Column::Breached)
            .column(ticket::Column::Satisfaction)
            .filter(condition)
            .scoped(tenant)
            .limit(MAX_ROWS)
            .into_tuple()
            .all(tenant.database().read())
            .await?;
        Ok(summarize(&rows))
    }
}

fn summarize(rows: &[Row]) -> Summary {
    let tickets = rows.len() as u64;
    let mut first: Vec<i64> = Vec::new();
    let mut resolve: Vec<i64> = Vec::new();
    let (mut solved, mut breached, mut rated, mut rating) = (0u64, 0u64, 0u64, 0i64);
    for (created, responded, solved_at, late, satisfaction) in rows {
        if let Some(responded) = responded {
            first.push((*responded - *created).num_minutes());
        }
        if let Some(solved_at) = solved_at {
            solved += 1;
            resolve.push((*solved_at - *created).num_minutes());
        }
        if *late {
            breached += 1;
        }
        if let Some(value) = satisfaction {
            rated += 1;
            rating += i64::from(*value);
        }
    }
    Summary {
        tickets,
        solved,
        breached,
        breach_rate_bp: (breached * 10_000).checked_div(tickets).unwrap_or(0),
        median_first_response_minutes: median(&mut first),
        median_resolve_minutes: median(&mut resolve),
        satisfaction_count: rated,
        satisfaction_avg_x100: (rated > 0).then(|| rating * 100 / rated as i64),
    }
}

fn median(values: &mut [i64]) -> Option<i64> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let middle = values.len() / 2;
    Some(if values.len().is_multiple_of(2) {
        (values[middle - 1] + values[middle]) / 2
    } else {
        values[middle]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trung_vi_va_ti_le() {
        assert_eq!(median(&mut [5, 1, 3]), Some(3));
        assert_eq!(median(&mut [4, 1, 3, 2]), Some(2));
        assert_eq!(median(&mut []), None);
        let at = Utc::now();
        let rows = [
            (at, Some(at + chrono::Duration::minutes(10)), None, true, Some(4)),
            (at, None, Some(at + chrono::Duration::minutes(60)), false, Some(5)),
        ];
        let summary = summarize(&rows);
        assert_eq!((summary.tickets, summary.breach_rate_bp), (2, 5000));
        assert_eq!(summary.satisfaction_avg_x100, Some(450));
        assert_eq!(summary.median_first_response_minutes, Some(10));
    }
}

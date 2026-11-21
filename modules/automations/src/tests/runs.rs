//! Chạy luật: job ngoài transaction, nhận lại event, chờ duyệt, chống vòng lặp, chạy lại, chạy thử
//! khô, `wait`, giới hạn số lần chạy, người tạo mất quyền.

use autoxide_kernel::access::ActorKind;
use autoxide_kernel::event::Delivered;
use autoxide_testkit::TestDatabase;
use serde_json::json;
use uuid::Uuid;

use super::{Harness, OWNER, on_order, person};
use crate::runner::triggers::EventTrigger;
use crate::runner::{AWAITING_APPROVAL, FAILED, SKIPPED, SUCCEEDED, WAITING};
use crate::service::rules::PAUSED;

fn tag_action() -> serde_json::Value {
    json!([{"tool": "shop.tag_order", "arguments": {"order_id": "{{ event.code }}", "tag": "vip"}}])
}

#[tokio::test]
async fn luat_chay_khi_event_den_va_dieu_kien_dung() {
    TestDatabase::run(async |test| {
        let harness = Harness::new(test).await;
        let big = json!({"field": "event.total.amount", "op": "gte", "value": 5_000_000});
        let rule = harness.rule(on_order(), big, tag_action()).await;
        harness.shop.place(&harness.tenant, 100).await;
        harness.shop.place(&harness.tenant, 6_000_000).await;
        harness.settle().await;
        let runs = harness.runs(rule.id).await;
        // Chỉ đơn lớn kích hoạt; tham số điền từ event.
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, SUCCEEDED, "{:?}", runs[0].error_message);
        assert_eq!(harness.count(|c| &c.tagged), 1);
        let steps: serde_json::Value = serde_json::from_str(&runs[0].steps).unwrap();
        assert_eq!(steps[0]["result"], json!({"order_id": "DH1", "tag": "vip"}));
        assert_eq!(steps[0]["idempotency_key"], json!(format!("automation:{}:0", runs[0].id)));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nhan_lai_event_khong_chay_hai_lan() {
    TestDatabase::run(async |test| {
        let harness = Harness::new(test).await;
        let rule = harness.rule(on_order(), json!(null), tag_action()).await;
        let event = Delivered::new(
            harness.tenant.clone(),
            "shop",
            "shop.order_placed",
            json!({"code": "A"}),
        );
        let trigger = EventTrigger(harness.service.clone());
        use autoxide_kernel::event::EventHandler;
        trigger.handle(&event).await.unwrap();
        trigger.handle(&event).await.unwrap();
        // Hai lần giao đồng thời cũng chỉ một lần chạy.
        let again = Delivered::new(
            harness.tenant.clone(),
            "shop",
            "shop.order_placed",
            json!({"code": "B"}),
        );
        let (first, second) = tokio::join!(trigger.handle(&again), trigger.handle(&again));
        first.unwrap();
        second.unwrap();
        harness.settle().await;
        assert_eq!(harness.runs(rule.id).await.len(), 2);
        assert_eq!(harness.count(|c| &c.tagged), 2);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hanh_dong_rui_ro_cho_duyet() {
    TestDatabase::run(async |test| {
        let harness = Harness::new(test).await;
        let actions = json!([{"tool": "shop.purge"}, {"tool": "shop.tag_order", "arguments": {"order_id": "1", "tag": "x"}}]);
        let rule = harness.rule(on_order(), json!(null), actions).await;
        harness.shop.place(&harness.tenant, 100).await;
        harness.settle().await;
        let run = harness.runs(rule.id).await.remove(0);
        assert_eq!(run.status, AWAITING_APPROVAL);
        assert_eq!(run.approval_step, Some(0));
        assert_eq!(harness.count(|c| &c.purged), 0);
        // AI agent không duyệt được; người thiếu quyền của tool cũng không.
        let agent = harness.owner.clone().with_actor_kind(ActorKind::Agent);
        let error = harness.service.approve_run(&harness.tenant, &agent, run.id).await.unwrap_err();
        assert_eq!(error.code(), "automations.human_required");
        let clerk = person(4, &["automations.read", "automations.run"]);
        let error = harness.service.approve_run(&harness.tenant, &clerk, run.id).await.unwrap_err();
        assert_eq!(error.code(), "permission_denied");
        // Người có quyền duyệt: bước chạy đúng một lần, các bước sau chạy tiếp.
        harness.service.approve_run(&harness.tenant, &harness.owner, run.id).await.unwrap();
        harness.settle().await;
        let run = harness.service.get_run(&harness.tenant, &harness.owner, run.id).await.unwrap();
        assert_eq!(run.status, SUCCEEDED, "{:?}", run.error_message);
        assert_eq!(harness.count(|c| &c.purged), 1);
        assert_eq!(harness.count(|c| &c.tagged), 1);
        let steps: serde_json::Value = serde_json::from_str(&run.steps).unwrap();
        assert_eq!(steps[0]["approved_by"], json!(harness.owner.user_id()));
        // Lần chạy sau lại chờ duyệt; từ chối thì lần chạy hỏng, không làm gì.
        harness.shop.place(&harness.tenant, 100).await;
        harness.settle().await;
        let second = harness.runs(rule.id).await.remove(1);
        assert_eq!(second.status, AWAITING_APPROVAL);
        let rejected = harness.service.reject_run(&harness.tenant, &harness.owner, second.id).await.unwrap();
        assert_eq!(rejected.status, FAILED);
        assert_eq!(rejected.error_code.as_deref(), Some("automations.approval_rejected"));
        assert_eq!(harness.count(|c| &c.purged), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn luat_khong_tu_kich_hoat_lai_minh() {
    TestDatabase::run(async |test| {
        let harness = Harness::new(test).await;
        // Luật nghe "có đơn" và hành động tạo thêm đơn: không được tự chạy mãi.
        let rule =
            harness.rule(on_order(), json!(null), json!([{"tool": "shop.place_order"}])).await;
        // Luật thứ hai cũng nghe "có đơn": chuỗi A → B được, nhưng có giới hạn độ sâu.
        let other =
            harness.rule(on_order(), json!(null), json!([{"tool": "shop.place_order"}])).await;
        harness.shop.place(&harness.tenant, 100).await;
        harness.settle().await;
        let runs = harness.runs(rule.id).await;
        let skipped: Vec<_> = runs.iter().filter(|run| run.status == SKIPPED).collect();
        assert!(!skipped.is_empty());
        assert!(
            skipped
                .iter()
                .all(|run| run.error_code.as_deref() == Some("automations.loop_detected"))
        );
        // Mỗi lần chạy thật có độ sâu tối đa 3; tổng số đơn tạo ra hữu hạn.
        let mut all = runs.clone();
        all.extend(harness.runs(other.id).await);
        assert!(
            all.iter()
                .filter(|run| run.status == SUCCEEDED)
                .all(|run| run.depth <= crate::config::MAX_CHAIN_DEPTH)
        );
        // Đơn gốc → A, B (độ sâu 1) → B, A (độ sâu 2) → A, B (độ sâu 3) → dừng: 6 đơn.
        assert_eq!(harness.count(|c| &c.placed), 6);
        // Một luật một mình: chạy đúng một lần cho đơn của người, đơn nó tạo không kích hoạt lại nó.
        harness
            .service
            .disable_rule(&harness.tenant, &harness.owner, other.id, None)
            .await
            .unwrap();
        let before = harness.count(|c| &c.placed);
        harness.shop.place(&harness.tenant, 100).await;
        harness.settle().await;
        assert_eq!(harness.count(|c| &c.placed), before + 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn chay_lai_sau_loi_khong_lam_hai_lan() {
    TestDatabase::run(async |test| {
        let harness = Harness::new(test).await;
        let actions = json!([
            {"tool": "shop.tag_order", "arguments": {"order_id": "1", "tag": "x"}},
            {"tool": "shop.flaky"}
        ]);
        let rule = harness.rule(on_order(), json!(null), actions).await;
        harness.shop.place(&harness.tenant, 100).await;
        harness.settle().await;
        let run = harness.runs(rule.id).await.remove(0);
        assert_eq!(run.status, FAILED);
        assert_eq!(run.error_code.as_deref(), Some("shop.busy"));
        assert_eq!(run.cursor, 1);
        harness.service.retry_run(&harness.tenant, &harness.owner, run.id).await.unwrap();
        harness.settle().await;
        let run = harness.service.get_run(&harness.tenant, &harness.owner, run.id).await.unwrap();
        assert_eq!(run.status, SUCCEEDED);
        // Bước đầu không chạy lại; bước hỏng chạy lại một lần.
        assert_eq!(harness.count(|c| &c.tagged), 1);
        assert_eq!(harness.count(|c| &c.flaky), 2);
        // Lần chạy đã xong thì không chạy lại được.
        let error =
            harness.service.retry_run(&harness.tenant, &harness.owner, run.id).await.unwrap_err();
        assert_eq!(error.code(), "automations.run_state");
        // Máy chủ chết sau khi tool chạy xong mà chưa kịp ghi bước: lần chạy lại lấy kết quả đã lưu
        // theo `Idempotency-Key` của bước, không gọi tool lần hai.
        {
            use autoxide_kernel::tenancy::ScopeExt;
            use sea_orm::sea_query::Expr;
            use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
            crate::entity::run::Entity::update_many()
                .col_expr(crate::entity::run::Column::Status, Expr::value("queued"))
                .col_expr(crate::entity::run::Column::Cursor, Expr::value(0))
                .col_expr(crate::entity::run::Column::Steps, Expr::value("[]"))
                .filter(crate::entity::run::Column::Id.eq(run.id))
                .scoped(&harness.tenant)
                .exec(harness.tenant.database().write())
                .await
                .unwrap();
        }
        harness.service.execute(&harness.tenant, run.id).await.unwrap();
        let replayed =
            harness.service.get_run(&harness.tenant, &harness.owner, run.id).await.unwrap();
        assert_eq!(replayed.status, SUCCEEDED);
        assert_eq!(harness.count(|c| &c.tagged), 1);
        assert_eq!(harness.count(|c| &c.flaky), 2);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn chay_thu_kho_khong_goi_tool() {
    TestDatabase::run(async |test| {
        let harness = Harness::new(test).await;
        let conditions = json!({"field": "event.total", "op": "gt", "value": 10});
        let actions = json!([
            {"tool": "shop.tag_order", "arguments": {"order_id": "{{ event.code }}", "tag": "{{ event.tag }}"}},
            {"tool": "shop.purge"},
            {"stop_if": {"field": "event.total", "op": "gt", "value": 100}},
            {"notify": {"title": "Order {{ event.code }}", "body": "Total {{ event.total }}"}}
        ]);
        let rule = harness.rule(on_order(), conditions, actions).await;
        let sample = json!({"code": "DH9", "total": 50, "tag": "a-tag-that-is-much-too-long"});
        let dry = harness.service.test_rule(&harness.tenant, &harness.owner, rule.id, Some(sample)).await.unwrap();
        assert!(dry.matched);
        assert!(!dry.ok);
        assert_eq!(dry.steps[0].arguments, Some(json!({"order_id": "DH9", "tag": "a-tag-that-is-much-too-long"})));
        assert_eq!(dry.steps[0].problems[0].field, "actions.0.arguments.tag");
        assert!(dry.steps[1].needs_approval);
        assert_eq!(dry.steps[2].would_stop, Some(false));
        assert_eq!(dry.steps[3].arguments, Some(json!({"title": "Order DH9", "body": "Total 50"})));
        // Không tool nào được gọi, không lần chạy nào được ghi.
        assert_eq!(harness.count(|c| &c.tagged) + harness.count(|c| &c.purged), 0);
        assert!(harness.runs(rule.id).await.is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn wait_hen_gio_roi_chay_tiep() {
    TestDatabase::run(async |test| {
        let harness = Harness::new(test).await;
        let actions = json!([
            {"wait": {"minutes": 60}},
            {"tool": "shop.tag_order", "arguments": {"order_id": "1", "tag": "x"}}
        ]);
        let rule = harness.rule(json!({"type": "manual"}), json!(null), actions).await;
        let run = harness
            .service
            .run_rule(&harness.tenant, &harness.owner, rule.id, json!({}))
            .await
            .unwrap();
        harness.settle().await;
        let waiting =
            harness.service.get_run(&harness.tenant, &harness.owner, run.id).await.unwrap();
        assert_eq!(waiting.status, WAITING);
        let resume_at = waiting.resume_at.unwrap();
        assert!(resume_at > chrono::Utc::now() + chrono::Duration::minutes(59));
        // Chưa tới giờ: worker không chạy tiếp.
        harness.settle().await;
        assert_eq!(harness.count(|c| &c.tagged), 0);
        // Tới giờ (job hẹn giờ chạy): chạy tiếp từ bước sau `wait`.
        harness.service.execute(&harness.tenant, run.id).await.unwrap();
        let done = harness.service.get_run(&harness.tenant, &harness.owner, run.id).await.unwrap();
        assert_eq!(done.status, SUCCEEDED);
        assert_eq!(harness.count(|c| &c.tagged), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn vuot_so_lan_chay_tam_dung_luat() {
    TestDatabase::run(async |test| {
        let harness = Harness::with(test, &[("AUTOXIDE_AUTOMATIONS_MAX_RUNS_PER_HOUR", "2")]).await;
        let rule = harness.rule(on_order(), json!(null), tag_action()).await;
        for _ in 0..3 {
            harness.shop.place(&harness.tenant, 100).await;
        }
        harness.settle().await;
        assert_eq!(harness.runs(rule.id).await.len(), 2);
        let paused = harness.reload(rule.id).await;
        assert_eq!(paused.status, PAUSED);
        assert_eq!(paused.status_reason.as_deref(), Some("rate_limit"));
        // Chạy tay luật đang tạm dừng: 409.
        let error = harness
            .service
            .run_rule(&harness.tenant, &harness.owner, rule.id, json!({}))
            .await
            .unwrap_err();
        assert_eq!(error.code(), "automations.rule_paused");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nguoi_tao_mat_quyen_thi_luat_tu_tat() {
    TestDatabase::run(async |test| {
        let harness = Harness::new(test).await;
        let rule = harness.rule(on_order(), json!(null), tag_action()).await;
        // Người tạo mất quyền `shop.write`.
        let weaker: Vec<&str> =
            OWNER.iter().copied().filter(|name| *name != "shop.write").collect();
        harness.people.set(person(1, &weaker));
        harness.shop.place(&harness.tenant, 100).await;
        harness.settle().await;
        let run = harness.runs(rule.id).await.remove(0);
        assert_eq!(run.status, FAILED);
        assert_eq!(run.error_code.as_deref(), Some("automations.permission_missing"));
        assert_eq!(harness.count(|c| &c.tagged), 0);
        let rule = harness.reload(rule.id).await;
        assert_eq!(rule.status, PAUSED);
        assert_eq!(rule.status_reason.as_deref(), Some("permission_lost"));
        // Tài khoản không còn: lý do khác.
        let other = harness.rule(on_order(), json!(null), tag_action()).await;
        harness.people.set(person(1, &OWNER));
        harness.people.remove(Uuid::from_u128(1));
        harness.shop.place(&harness.tenant, 100).await;
        harness.settle().await;
        assert_eq!(
            harness.reload(other.id).await.status_reason.as_deref(),
            Some("owner_unavailable")
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn lich_chay_dung_moc_mot_lan() {
    TestDatabase::run(async |test| {
        use autoxide_kernel::tenancy::ScopeExt;
        use sea_orm::sea_query::Expr;
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
        let harness = Harness::new(test).await;
        let schedule =
            json!({"type": "schedule", "every": "day", "hour": 9, "timezone": "Asia/Ho_Chi_Minh"});
        let rule = harness.rule(schedule, json!(null), tag_action()).await;
        assert!(rule.next_run_at.is_some());
        // Đưa mốc về quá khứ: lần quét chạy luật một lần và dời mốc sang ngày sau.
        let past = chrono::Utc::now() - chrono::Duration::minutes(5);
        let past = chrono::SubsecRound::trunc_subsecs(past, 6);
        crate::entity::rule::Entity::update_many()
            .col_expr(crate::entity::rule::Column::NextRunAt, Expr::value(past))
            .filter(crate::entity::rule::Column::Id.eq(rule.id))
            .scoped(&harness.tenant)
            .exec(harness.tenant.database().write())
            .await
            .unwrap();
        harness.service.tick(harness.tenant.database()).await.unwrap();
        harness.service.tick(harness.tenant.database()).await.unwrap();
        harness.settle().await;
        let runs = harness.runs(rule.id).await;
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].trigger_key, format!("schedule:{}", past.to_rfc3339()));
        assert!(harness.reload(rule.id).await.next_run_at.unwrap() > chrono::Utc::now());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn chay_cung_luc_khong_vuot_so_lan_moi_gio() {
    TestDatabase::run(async |test| {
        let harness = Harness::with(test, &[("AUTOXIDE_AUTOMATIONS_MAX_RUNS_PER_HOUR", "2")]).await;
        let rule = harness.rule(json!({"type": "manual"}), json!(null), tag_action()).await;
        // Bốn lần chạy tay cùng lúc. Trước đây đếm số lần trong giờ không khoá: Postgres cho
        // nhiều bên cùng đếm thiếu và vượt `max_runs_per_hour`.
        let run = || harness.service.run_rule(&harness.tenant, &harness.owner, rule.id, json!({}));
        let _ = tokio::join!(run(), run(), run(), run());
        assert_eq!(harness.runs(rule.id).await.len(), 2);
        assert_eq!(harness.reload(rule.id).await.status, PAUSED);
    })
    .await
    .unwrap();
}

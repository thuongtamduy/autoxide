//! Chạy luật: job ngoài transaction, nhận lại event, chờ duyệt, chống vòng lặp, chạy lại, chạy thử
//! khô, `wait`, giới hạn số lần chạy, người tạo mất quyền.

use autoxide_kernel::access::ActorKind;
use autoxide_kernel::event::Delivered;
use autoxide_kernel::time::Timestamp;
use autoxide_testkit::TestDatabase;
use jiff::SignedDuration;
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

/// L-8 (rà soát bảo mật 2026-11): luật của một nút con chỉ chạy với event thuộc nút đó — event của nút
/// khác hay event không nêu nút thì không; luật của nút gốc vẫn nhận mọi event của khách.
#[tokio::test]
async fn luat_cua_nut_con_chi_nhan_event_cua_nut_do() {
    TestDatabase::run(async |test| {
        use autoxide_kernel::event::EventHandler;
        use autoxide_kernel::tenancy::ScopeExt;
        use sea_orm::sea_query::Expr;
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

        let harness = Harness::new(test).await;
        let root_rule = harness.rule(on_order(), json!(null), tag_action()).await;
        let branch_rule = harness.rule(on_order(), json!(null), tag_action()).await;
        let branch = Uuid::now_v7();
        crate::entity::rule::Entity::update_many()
            .col_expr(crate::entity::rule::Column::OrgId, Expr::value(branch))
            .filter(crate::entity::rule::Column::Id.eq(branch_rule.id))
            .scoped(&harness.tenant)
            .exec(harness.tenant.database().write())
            .await
            .unwrap();

        let trigger = EventTrigger(harness.service.clone());
        let other = Uuid::now_v7().to_string();
        for (code, payload) in [
            ("own", json!({"code": "1", "org_id": branch.to_string()})),
            ("own-after", json!({"code": "2", "after": {"org_id": branch.to_string()}})),
            ("other", json!({"code": "3", "org_id": other})),
            ("none", json!({"code": "4"})),
        ] {
            let event =
                Delivered::new(harness.tenant.clone(), "shop", "shop.order_placed", payload);
            trigger.handle(&event).await.unwrap_or_else(|error| panic!("{code}: {error}"));
        }
        assert_eq!(harness.runs(root_rule.id).await.len(), 4, "luật nút gốc nhận mọi event");
        assert_eq!(
            harness.runs(branch_rule.id).await.len(),
            2,
            "luật nút con chỉ nhận event của nút đó"
        );
    })
    .await
    .unwrap();
}

/// L-8 với cây tổ chức thật (module `organizations`): luật của nút `north` chạy với event của chính nó và của nút
/// con `north-shop`, không chạy với event của nút anh em `south` hay của nút cha (gốc).
#[tokio::test]
async fn luat_cua_nut_con_theo_cay_to_chuc() {
    TestDatabase::run(async |test| {
        use autoxide_kernel::event::EventHandler;
        use autoxide_kernel::tenancy::{OrgId, OrgTree, ScopeExt};
        use sea_orm::sea_query::Expr;
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

        let harness = Harness::with_orgs(test).await;
        let tenant = harness.tenant.id();
        let (north, north_shop, south) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let write = harness.tenant.database().begin_write().await.unwrap();
        let root = OrgTree::insert_root(&write, tenant).await.unwrap();
        OrgTree::insert(&write, tenant, OrgId::from_uuid(north), root, 8).await.unwrap();
        OrgTree::insert(&write, tenant, OrgId::from_uuid(north_shop), OrgId::from_uuid(north), 8)
            .await
            .unwrap();
        OrgTree::insert(&write, tenant, OrgId::from_uuid(south), root, 8).await.unwrap();
        write.commit().await.unwrap();

        let rule = harness.rule(on_order(), json!(null), tag_action()).await;
        crate::entity::rule::Entity::update_many()
            .col_expr(crate::entity::rule::Column::OrgId, Expr::value(north))
            .filter(crate::entity::rule::Column::Id.eq(rule.id))
            .scoped(&harness.tenant)
            .exec(harness.tenant.database().write())
            .await
            .unwrap();
        let trigger = EventTrigger(harness.service.clone());
        for (code, org) in
            [("north", north), ("shop", north_shop), ("south", south), ("root", root.as_uuid())]
        {
            let payload = json!({"code": code, "org_id": org.to_string()});
            let event =
                Delivered::new(harness.tenant.clone(), "shop", "shop.order_placed", payload);
            trigger.handle(&event).await.unwrap();
        }
        let runs = harness.runs(rule.id).await;
        let mut codes: Vec<String> = runs
            .iter()
            .map(|run| {
                serde_json::from_str::<serde_json::Value>(&run.context).unwrap()["event"]["code"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        codes.sort();
        assert_eq!(codes, ["north", "shop"], "chỉ nút của luật và nhánh con");
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
        assert!(resume_at > Timestamp::now() + SignedDuration::from_mins(59));
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

/// D-20261126-automation-owner-revoked: lần chạy đang chờ (`wait`) không chạy tiếp bằng quyền của `run_as` khi luật
/// đã tạm dừng trong lúc chờ (chủ luật bị cắt cửa vào) — dừng với `automations.rule_not_active`, không gọi tool.
#[tokio::test]
async fn lan_chay_dang_cho_dung_khi_luat_tam_dung() {
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
        harness.service.owner_revoked(&harness.tenant, harness.owner.user_id()).await.unwrap();
        harness.service.execute(&harness.tenant, run.id).await.unwrap();
        let stopped =
            harness.service.get_run(&harness.tenant, &harness.owner, run.id).await.unwrap();
        assert_eq!(stopped.status, FAILED);
        assert_eq!(stopped.error_code.as_deref(), Some("automations.rule_not_active"));
        assert_eq!(harness.count(|c| &c.tagged), 0);
    })
    .await
    .unwrap();
}

/// Lần chạy đang chờ của luật bị tạm dừng không sống lại khi người khác bật lại luật: nó đã kết thúc lúc tạm dừng, và
/// job tới hạn không chạy tool nào (D-20261126-automation-owner-revoked).
#[tokio::test]
async fn bat_lai_luat_khong_chay_tiep_lan_chay_cu() {
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
        harness.service.owner_revoked(&harness.tenant, harness.owner.user_id()).await.unwrap();
        let closed =
            harness.service.get_run(&harness.tenant, &harness.owner, run.id).await.unwrap();
        assert_eq!(closed.status, FAILED, "lần chạy chờ kết thúc cùng lúc tạm dừng");
        assert_eq!(closed.error_code.as_deref(), Some("automations.rule_not_active"));
        // Người khác bật lại luật, rồi job hẹn giờ tới hạn.
        let boss = person(9, &OWNER);
        harness.people.set(boss.clone());
        harness.service.enable_rule(&harness.tenant, &boss, rule.id, None).await.unwrap();
        harness.service.execute(&harness.tenant, run.id).await.unwrap();
        harness.settle().await;
        assert_eq!(
            harness.count(|c| &c.tagged),
            0,
            "không chạy tool theo định nghĩa cũ bằng quyền người mới"
        );
    })
    .await
    .unwrap();
}

/// Lưới thứ hai: lần chạy đang chờ của luật còn bật nhưng vừa bị sửa (phiên bản đổi) không chạy tiếp định nghĩa cũ.
#[tokio::test]
async fn sua_luat_trong_luc_cho_thi_lan_chay_cu_dung() {
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
        let changes = crate::service::rules::RuleChanges {
            name: Some("renamed".to_owned()),
            description: None,
            trigger: None,
            conditions: None,
            actions: None,
        };
        harness
            .service
            .update_rule(&harness.tenant, &harness.owner, rule.id, changes, None)
            .await
            .unwrap();
        harness.service.execute(&harness.tenant, run.id).await.unwrap();
        let stopped =
            harness.service.get_run(&harness.tenant, &harness.owner, run.id).await.unwrap();
        assert_eq!(stopped.status, FAILED);
        assert_eq!(stopped.error_code.as_deref(), Some("automations.rule_changed"));
        assert_eq!(harness.count(|c| &c.tagged), 0);
    })
    .await
    .unwrap();
}

/// Luật bị tạm dừng giữa hai bước tool liền nhau (không có `wait`): bước sau không chạy.
#[tokio::test]
async fn tam_dung_giua_hai_buoc_tool() {
    TestDatabase::run(async |test| {
        let harness = Harness::new(test).await;
        let actions = json!([
            {"tool": "shop.tag_order", "arguments": {"order_id": "1", "tag": "pause-rules"}},
            {"tool": "shop.tag_order", "arguments": {"order_id": "1", "tag": "x"}}
        ]);
        let rule = harness.rule(json!({"type": "manual"}), json!(null), actions).await;
        let run = harness
            .service
            .run_rule(&harness.tenant, &harness.owner, rule.id, json!({}))
            .await
            .unwrap();
        harness.settle().await;
        let stopped =
            harness.service.get_run(&harness.tenant, &harness.owner, run.id).await.unwrap();
        assert_eq!(stopped.status, FAILED);
        assert_eq!(stopped.error_code.as_deref(), Some("automations.rule_not_active"));
        assert_eq!(harness.count(|c| &c.tagged), 1, "chỉ bước đầu chạy");
    })
    .await
    .unwrap();
}

/// Lần chạy đang `running` (không đóng được lúc tạm dừng) mà luật bị tạm dừng rồi người khác bật lại ngay giữa chừng:
/// trước bước kế tiếp (bước `wait`) runner thấy phiên bản đổi và dừng với `automations.rule_changed` — không bước nào
/// chạy tiếp, kể cả khi tới hạn.
#[tokio::test]
async fn dang_chay_ma_luat_duoc_bat_lai_thi_dung() {
    TestDatabase::run(async |test| {
        let harness = Harness::new(test).await;
        let actions = json!([
            {"tool": "shop.tag_order", "arguments": {"order_id": "1", "tag": "reenable-rules"}},
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
        harness.service.execute(&harness.tenant, run.id).await.unwrap();
        let stopped =
            harness.service.get_run(&harness.tenant, &harness.owner, run.id).await.unwrap();
        assert_eq!(stopped.status, FAILED);
        assert_eq!(stopped.error_code.as_deref(), Some("automations.rule_changed"));
        assert_eq!(harness.count(|c| &c.tagged), 1, "chỉ bước đầu chạy");
    })
    .await
    .unwrap();
}

/// `start_run` xem lại luật dưới khoá: bên gọi cầm bản luật cũ (đọc trễ) của luật vừa tạm dừng thì không có lần chạy
/// mới; xoá luật thì lần chạy đang chờ kết thúc cùng lúc.
#[tokio::test]
async fn khong_tao_lan_chay_cho_luat_vua_tam_dung_va_xoa_luat_dong_lan_chay() {
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
        harness.service.delete_rule(&harness.tenant, &harness.owner, rule.id, None).await.unwrap();
        let closed =
            harness.service.get_run(&harness.tenant, &harness.owner, run.id).await.unwrap();
        assert_eq!(closed.status, FAILED);
        assert_eq!(closed.error_code.as_deref(), Some("automations.rule_not_active"));

        let other = harness.rule(on_order(), json!(null), tag_action()).await;
        let stale_other = harness.reload(other.id).await;
        harness.service.owner_revoked(&harness.tenant, harness.owner.user_id()).await.unwrap();
        let start = crate::runner::Start {
            trigger_key: "manual:stale".to_owned(),
            event: None,
            context: json!({}),
            depth: 1,
            triggered_by: None,
            skip: None,
        };
        let created =
            harness.service.start_run(&harness.tenant, &stale_other, start).await.unwrap();
        assert!(created.is_none(), "luật đã tạm dừng không nhận lần chạy mới");
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
        let past = Timestamp::now() - SignedDuration::from_mins(5);
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
        assert!(harness.reload(rule.id).await.next_run_at.unwrap() > Timestamp::now());
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

/// Rà soát bảo mật 2026-11: duyệt bước dùng permission `money` cần phiên vừa xác nhận lại, như gọi
/// REST — người duyệt có quyền nhưng phiên cũ thì `step_up_required`.
#[tokio::test]
async fn duyet_buoc_nhay_cam_doi_xac_nhan_lai() {
    use autoxide_kernel::access::StepUp;
    use autoxide_kernel::time::Timestamp;
    use jiff::SignedDuration;

    TestDatabase::run(async |test| {
        let harness = Harness::new(test).await;
        let mut permissions = OWNER.to_vec();
        permissions.push("shop.refund");
        let window = SignedDuration::from_mins(15);
        let at = |minutes: i64| StepUp {
            authenticated_at: Some(Timestamp::now() - SignedDuration::from_mins(minutes)),
            window,
        };
        let fresh = person(6, &permissions).with_step_up(at(0));
        // Lần chạy lấy quyền hiện tại của người tạo qua bộ xác thực của test.
        harness.people.set(person(6, &permissions));
        let rule = harness
            .rule_by(&fresh, on_order(), json!(null), json!([{"tool": "shop.refund"}]))
            .await
            .unwrap();
        harness.shop.place(&harness.tenant, 100).await;
        harness.settle().await;
        let run = harness.runs(rule.id).await.remove(0);
        assert_eq!(run.status, AWAITING_APPROVAL);
        let stale = person(6, &permissions).with_step_up(at(60));
        let error = harness.service.approve_run(&harness.tenant, &stale, run.id).await.unwrap_err();
        assert_eq!(error.code(), "step_up_required");
        harness.service.approve_run(&harness.tenant, &fresh, run.id).await.unwrap();
    })
    .await
    .unwrap();
}

/// Số event `automations.run_failed` trong outbox.
async fn run_failed_events(harness: &Harness) -> i64 {
    use sea_orm::ConnectionTrait;
    let read = harness.tenant.database().read();
    let backend = read.get_database_backend();
    let sql = match backend {
        sea_orm::DatabaseBackend::Postgres => {
            "SELECT COUNT(*) AS n FROM automations_events WHERE name = $1"
        }
        _ => "SELECT COUNT(*) AS n FROM automations_events WHERE name = ?",
    };
    let statement =
        autoxide_kernel::database::statement(backend, sql, ["automations.run_failed".into()]);
    let row = read.query_one_raw(statement).await.unwrap().unwrap();
    row.try_get::<i64>("", "n").unwrap()
}

/// Đóng lần chạy đang chờ (luật tạm dừng) cùng lúc worker nhận nó: lần chạy kết thúc đúng một lần — hoặc worker chạy
/// xong (tool gọi đúng một lần, không event hỏng), hoặc hỏng với đúng một event `automations.run_failed` — không bao
/// giờ hỏng hai lần hay còn treo.
#[tokio::test]
async fn dong_lan_chay_cung_luc_worker_nhan() {
    TestDatabase::run(async |test| {
        let harness = Harness::new(test).await;
        let actions = json!([
            {"wait": {"minutes": 60}},
            {"tool": "shop.tag_order", "arguments": {"order_id": "1", "tag": "x"}}
        ]);
        for round in 0..5 {
            let rule = harness.rule(json!({"type": "manual"}), json!(null), actions.clone()).await;
            let run = harness
                .service
                .run_rule(&harness.tenant, &harness.owner, rule.id, json!({}))
                .await
                .unwrap();
            harness.settle().await;
            let failed_before = run_failed_events(&harness).await;
            let tagged_before = harness.count(|c| &c.tagged);
            let rule = harness.reload(rule.id).await;
            let (closed, worked) = tokio::join!(
                harness.service.pause_now(&harness.tenant, &rule, "test"),
                harness.service.execute(&harness.tenant, run.id),
            );
            closed.unwrap();
            worked.unwrap();
            let done =
                harness.service.get_run(&harness.tenant, &harness.owner, run.id).await.unwrap();
            let failed = run_failed_events(&harness).await - failed_before;
            let tagged = harness.count(|c| &c.tagged) - tagged_before;
            match done.status.as_str() {
                SUCCEEDED => assert_eq!((failed, tagged), (0, 1), "round {round}"),
                FAILED => assert_eq!((failed, tagged), (1, 0), "round {round}"),
                other => panic!("round {round}: run left `{other}`"),
            }
        }
    })
    .await
    .unwrap();
}

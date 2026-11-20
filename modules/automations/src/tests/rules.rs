//! Lưu luật: kiểm một lần mọi lỗi, quyền của mọi tool, luật của agent chờ duyệt, phạm vi theo nút.

use autoxide_kernel::access::{ActorKind, Scope};
use autoxide_kernel::pagination::{Order, PageRequest};
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree, root_of};
use autoxide_testkit::TestDatabase;
use serde_json::json;
use uuid::Uuid;

use super::{Harness, on_order, person};
use crate::service::rules::{ACTIVE, NewRule, PENDING_APPROVAL, RuleFilter};
use crate::service::validate::RawBody;

#[tokio::test]
async fn luu_luat_kiem_quyen_moi_tool() {
    TestDatabase::run(async |test| {
        let harness = Harness::new(test).await;
        // Người chỉ có quyền quản lý luật, không có quyền của tool trong luật.
        let manager = person(2, &["automations.read", "automations.manage"]);
        let error = harness
            .rule_by(
                &manager,
                json!({"type": "event", "event": "shop.no_such_event"}),
                json!({"field": "event.total", "op": "gt", "value": 1}),
                json!([
                    {"tool": "shop.tag_order", "arguments": {"order_id": "{{ event.code }}", "tag": "vip"}},
                    {"tool": "shop.nothing"},
                    {"tool": "automations.create_rule"}
                ]),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code(), "validation_failed");
        let problems: Vec<(String, String)> =
            error.fields().iter().map(|field| (field.field.clone(), field.code.clone())).collect();
        // Mọi lỗi báo một lần (D-083): event lạ, tool thiếu quyền, tool lạ, tool của chính module.
        assert_eq!(
            problems,
            vec![
                ("trigger.event".to_owned(), "automations.trigger_unknown".to_owned()),
                ("actions.0.tool".to_owned(), "automations.permission_missing".to_owned()),
                ("actions.1.tool".to_owned(), "automations.tool_unknown".to_owned()),
                ("actions.2.tool".to_owned(), "automations.tool_not_allowed".to_owned()),
            ]
        );
        // Người có quyền: tham số sai schema của tool cũng bị báo, mẫu dùng biến lạ cũng vậy.
        let error = harness
            .rule_by(
                &harness.owner,
                on_order(),
                json!(null),
                json!([{"tool": "shop.tag_order", "arguments": {"tag": "a-very-long-tag-name-over-20", "x": "{{ secret }}"}}]),
            )
            .await
            .unwrap_err();
        let fields: Vec<&str> = error.fields().iter().map(|field| field.field.as_str()).collect();
        assert!(fields.contains(&"actions.0.arguments.order_id"), "{fields:?}");
        assert!(fields.contains(&"actions.0.arguments.tag"), "{fields:?}");
        assert!(fields.contains(&"actions.0.arguments.x"), "{fields:?}");
        // Luật đúng thì lưu được, chạy với quyền người tạo.
        let rule = harness
            .rule(on_order(), json!({}), json!([{"tool": "shop.tag_order", "arguments": {"order_id": "{{ event.code }}", "tag": "vip"}}]))
            .await;
        assert_eq!(rule.status, ACTIVE);
        assert_eq!(rule.run_as, harness.owner.user_id());
        assert_eq!(rule.trigger_ref.as_deref(), Some("shop.order_placed"));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn agent_tao_luat_thi_cho_duyet() {
    TestDatabase::run(async |test| {
        let harness = Harness::new(test).await;
        let agent = harness.owner.clone().with_actor_kind(ActorKind::Agent);
        let actions =
            json!([{"tool": "shop.tag_order", "arguments": {"order_id": "1", "tag": "x"}}]);
        let rule = harness.rule_by(&agent, on_order(), json!(null), actions).await.unwrap();
        assert_eq!(rule.status, PENDING_APPROVAL);
        // Luật chờ duyệt không chạy.
        harness.shop.place(&harness.tenant, 100).await;
        harness.settle().await;
        assert!(harness.runs(rule.id).await.is_empty());
        // Agent không tự bật được.
        let error =
            harness.service.enable_rule(&harness.tenant, &agent, rule.id, None).await.unwrap_err();
        assert_eq!(error.code(), "automations.human_required");
        // Người bật: luật chạy với quyền của người đó.
        let human = super::person(3, &super::OWNER);
        harness.people.set(human.clone());
        let enabled =
            harness.service.enable_rule(&harness.tenant, &human, rule.id, None).await.unwrap();
        assert_eq!(enabled.status, ACTIVE);
        assert_eq!(enabled.run_as, human.user_id());
        // Agent sửa thân luật: lại chờ duyệt.
        let changes = crate::service::rules::RuleChanges {
            actions: Some(
                json!([{"tool": "shop.tag_order", "arguments": {"order_id": "2", "tag": "y"}}]),
            ),
            ..Default::default()
        };
        let changed = harness
            .service
            .update_rule(&harness.tenant, &agent, rule.id, changes, None)
            .await
            .unwrap();
        assert_eq!(changed.status, PENDING_APPROVAL);
        assert_eq!(changed.version, enabled.version + 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn luat_cua_nut_khac_khong_thay() {
    TestDatabase::run(async |test| {
        let harness = Harness::new(test).await;
        let root = root_of(DEFAULT_TENANT);
        let (a, b) = (OrgId::from_uuid(Uuid::now_v7()), OrgId::from_uuid(Uuid::now_v7()));
        let write = harness.app.database().begin_write().await.unwrap();
        let inserted_root = OrgTree::insert_root(&write, DEFAULT_TENANT).await.unwrap();
        OrgTree::insert(&write, DEFAULT_TENANT, a, inserted_root, 8).await.unwrap();
        OrgTree::insert(&write, DEFAULT_TENANT, b, inserted_root, 8).await.unwrap();
        write.commit().await.unwrap();
        assert_eq!(inserted_root, root);
        let input = NewRule {
            name: "Branch A".to_owned(),
            description: None,
            org_id: Some(a.as_uuid()),
            body: RawBody {
                trigger: json!({"type": "manual"}),
                conditions: json!(null),
                actions: json!([{"wait": {"minutes": 1}}]),
            },
            enabled: true,
        };
        let rule =
            harness.service.create_rule(&harness.tenant, &harness.owner, input).await.unwrap();
        let grants = |org: OrgId| {
            autoxide_kernel::access::Principal::with_grants(
                Uuid::from_u128(9),
                [(
                    "automations.read".to_owned(),
                    autoxide_kernel::access::Grant::new(org, Scope::Subtree),
                )],
            )
        };
        let page = PageRequest::first(50, Order::Desc);
        let filter = RuleFilter::default();
        // Người của nhánh B không thấy luật của nhánh A, đọc thẳng thì như không có.
        let south = grants(b);
        let listed =
            harness.service.list_rules(&harness.tenant, &south, &filter, &page).await.unwrap();
        assert!(listed.data.is_empty());
        let error = harness.service.get_rule(&harness.tenant, &south, rule.id).await.unwrap_err();
        assert_eq!(error.code(), "automations.rule_not_found");
        // Người của nhánh A thấy.
        let north = grants(a);
        let listed =
            harness.service.list_rules(&harness.tenant, &north, &filter, &page).await.unwrap();
        assert_eq!(listed.data.len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hai_lan_sua_dong_thoi_moi_lan_mot_phien_ban() {
    TestDatabase::run(async |test| {
        let harness = Harness::new(test).await;
        let actions =
            json!([{"tool": "shop.tag_order", "arguments": {"order_id": "1", "tag": "x"}}]);
        let rule = harness.rule_by(&harness.owner, on_order(), json!(null), actions).await.unwrap();
        let rename = |name: &str| crate::service::rules::RuleChanges {
            name: Some(name.to_owned()),
            ..Default::default()
        };
        // Hai lần sửa cùng lúc: Postgres từng cho cả hai cùng đọc phiên bản cũ và cùng ghi
        // `version + 1` — hai nội dung khác nhau mang cùng một số phiên bản.
        let (first, second) = tokio::join!(
            harness.service.update_rule(
                &harness.tenant,
                &harness.owner,
                rule.id,
                rename("A"),
                None
            ),
            harness.service.update_rule(
                &harness.tenant,
                &harness.owner,
                rule.id,
                rename("B"),
                None
            ),
        );
        let mut versions = [first.unwrap().version, second.unwrap().version];
        versions.sort_unstable();
        assert_eq!(versions, [rule.version + 1, rule.version + 2]);
    })
    .await
    .unwrap();
}

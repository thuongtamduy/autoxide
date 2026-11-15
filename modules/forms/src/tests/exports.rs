//! Xuất CSV lớn bằng job (đặc tả 32 B3): file giống từng byte bản đồng bộ, link ký, yêu cầu đồng
//! thời, nơi lưu lỗi, xoá theo email / xoá form / dọn theo hạn xoá cả file.

use std::sync::Arc;

use autoxide_kernel::storage::StoredFile;
use autoxide_testkit::MemoryStorage;
use bytes::Bytes;
use futures_util::TryStreamExt as _;

use super::*;
use crate::service::now;
use crate::submissions::exports::{ExportState, storage_key};
use crate::testing::service_with_storage;

/// Service với giới hạn gửi đủ rộng cho vài trăm bài từ một máy.
async fn exporting(
    test: &TestDatabase,
) -> (Arc<FormsService>, autoxide_kernel::database::Database, Arc<MemoryStorage>) {
    let config = FormsConfig {
        per_ip_per_hour: 100_000,
        per_email_per_hour: 100_000,
        ..FormsConfig::default()
    };
    let antispam = AntispamConfig::default().min_fill(Duration::ZERO);
    let (service, _, database, storage) = service_with_storage(test, config, antispam).await;
    (service, database, storage)
}

async fn bytes_of(file: StoredFile) -> Vec<u8> {
    let chunks: Vec<Bytes> = file.content.try_collect().await.unwrap();
    chunks.concat()
}

/// `(expires, signature)` trong link tải.
fn link_parts(url: &str) -> (i64, String) {
    let query = url.split_once('?').unwrap().1;
    let mut expires = 0;
    let mut signature = String::new();
    for pair in query.split('&') {
        match pair.split_once('=').unwrap() {
            ("expires", value) => expires = value.parse().unwrap(),
            ("signature", value) => signature = value.to_owned(),
            _ => {}
        }
    }
    (expires, signature)
}

/// Hơn một trang (200 bài): file của job giống từng byte bản xuất đồng bộ; link ký tải được;
/// sai chữ ký hay hết hạn bị từ chối; chạy lại job khi đã xong thì không đổi gì; có event.
#[tokio::test]
async fn xuat_qua_job_giong_tung_byte_ban_dong_bo() {
    TestDatabase::run(async |test| {
        let (service, database, storage) = exporting(test).await;
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let form = service.create_form(&tenant, &admin(), contact("contact")).await.unwrap();
        for index in 0..205 {
            let mut data = answers(&format!("Người {index}"), &format!("p{index}@example.com"));
            if index == 7 {
                data.insert("message".to_owned(), json!("=SUM(A1), \"trích\"\ndòng 2"));
            }
            send(&tenant, &service, "contact", &visitor("203.0.113.9"), data).await.unwrap();
        }
        let exporter = user(&["export"]);

        let view = service.request_export(&tenant, &exporter, form.row.id, None).await.unwrap();
        assert_eq!((view.state, view.download.is_none()), (ExportState::Pending, true));
        FormsService::run_export(&service, &tenant, view.row.id).await.unwrap();
        let ready = service.get_export(&tenant, &exporter, view.row.id).await.unwrap();
        assert_eq!(ready.state, ExportState::Ready);
        assert_eq!(ready.row.row_count, Some(205));
        let (_, text) = service.export_csv(&tenant, &exporter, form.row.id, None).await.unwrap();
        let stored = storage.bytes(&storage_key(view.row.id)).unwrap();
        assert_eq!(stored.as_ref(), text.as_bytes());
        assert_eq!(ready.row.size, Some(i64::try_from(text.len()).unwrap()));

        // Link ký: tải được, ra đúng file.
        let (url, _) = ready.download.clone().unwrap();
        assert!(url.starts_with(&format!("/api/v1/forms/exports/{}/file?", view.row.id)), "{url}");
        let (expires, signature) = link_parts(&url);
        let (name, file) =
            service.download_export(&tenant, view.row.id, expires, &signature).await.unwrap();
        assert_eq!(name, "contact-submissions.csv");
        assert_eq!(bytes_of(file).await, text.as_bytes());
        // Sai chữ ký, sửa hạn, chữ ký không phải hex, hạn đã qua: cùng một lỗi.
        let mut wrong = signature.clone();
        wrong.replace_range(0..2, if signature.starts_with("00") { "11" } else { "00" });
        let past = now().timestamp() - 1;
        let past_signature = service.signature(&tenant, view.row.id, past).await.unwrap();
        for (expires, signature) in [
            (expires, wrong.as_str()),
            (expires + 60, signature.as_str()),
            (expires, "not hex"),
            (past, past_signature.as_str()),
        ] {
            let result = service.download_export(&tenant, view.row.id, expires, signature).await;
            assert_eq!(
                result.err().map(|error| error.code().to_owned()).as_deref(),
                Some("forms.export_link_invalid")
            );
        }

        // Job chạy lại (hàng đợi thử lại): không ghi lại, không thêm event.
        FormsService::run_export(&service, &tenant, view.row.id).await.unwrap();
        let again = service.get_export(&tenant, &exporter, view.row.id).await.unwrap();
        assert_eq!(again.row.finished_at, ready.row.finished_at);
        let exported: Vec<_> = events(&database)
            .await
            .into_iter()
            .filter(|(name, _)| name == "forms.submissions_exported")
            .collect();
        assert_eq!(exported.len(), 1);
        assert!(exported[0].1.contains("\"row_count\":205"), "{}", exported[0].1);
        assert!(!exported[0].1.contains("example.com"), "no email in the event");

        // Lọc theo trạng thái như bản đồng bộ.
        let filtered = service
            .request_export(&tenant, &exporter, form.row.id, Some(SubmissionStatus::Spam))
            .await
            .unwrap();
        FormsService::run_export(&service, &tenant, filtered.row.id).await.unwrap();
        let filtered = service.get_export(&tenant, &exporter, filtered.row.id).await.unwrap();
        assert_eq!(filtered.row.row_count, Some(0));
        let (_, spam) = service
            .export_csv(&tenant, &exporter, form.row.id, Some(SubmissionStatus::Spam))
            .await
            .unwrap();
        assert_eq!(storage.bytes(&storage_key(filtered.row.id)).unwrap().as_ref(), spam.as_bytes());
    })
    .await
    .unwrap();
}

/// Hai yêu cầu xuất cùng lúc cho một form: một bên được, bên kia `forms.export_in_progress`.
/// Không có quyền `forms.export`: từ chối. Form không có: `forms.not_found`.
#[tokio::test]
async fn hai_yeu_cau_dong_thoi_chi_mot_ben_duoc() {
    TestDatabase::run(async |test| {
        let (service, database, _) = exporting(test).await;
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let form = service.create_form(&tenant, &admin(), contact("contact")).await.unwrap();
        let exporter = user(&["export"]);
        let (first, second) = tokio::join!(
            service.request_export(&tenant, &exporter, form.row.id, None),
            service.request_export(&tenant, &exporter, form.row.id, None),
        );
        let mut codes: Vec<&str> = [&first, &second]
            .iter()
            .map(|result| result.as_ref().map_or_else(|error| error.code(), |_| "ok"))
            .collect();
        codes.sort_unstable();
        assert_eq!(codes, ["forms.export_in_progress", "ok"]);
        let running = first.or(second).unwrap();
        // Xong rồi thì yêu cầu được bản mới.
        FormsService::run_export(&service, &tenant, running.row.id).await.unwrap();
        service.request_export(&tenant, &exporter, form.row.id, None).await.unwrap();

        for principal in [user(&["read"]), user(&["manage"])] {
            let error =
                service.request_export(&tenant, &principal, form.row.id, None).await.unwrap_err();
            assert_eq!(error.code(), "permission_denied");
            let error = service.get_export(&tenant, &principal, running.row.id).await.unwrap_err();
            assert_eq!(error.code(), "permission_denied");
        }
        let error =
            service.request_export(&tenant, &exporter, Uuid::now_v7(), None).await.unwrap_err();
        assert_eq!(error.code(), "forms.not_found");
        let error = service.get_export(&tenant, &exporter, Uuid::now_v7()).await.unwrap_err();
        assert_eq!(error.code(), "forms.export_not_found");
    })
    .await
    .unwrap();
}

/// Nơi lưu lỗi khi ghi: bản xuất `failed`, không có link; yêu cầu bản mới được ngay.
#[tokio::test]
async fn noi_luu_loi_thi_ban_xuat_that_bai() {
    TestDatabase::run(async |test| {
        let (service, database, storage) = exporting(test).await;
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let form = service.create_form(&tenant, &admin(), contact("contact")).await.unwrap();
        send(
            &tenant,
            &service,
            "contact",
            &visitor("203.0.113.9"),
            answers("An", "an@example.com"),
        )
        .await
        .unwrap();
        let exporter = user(&["export"]);
        let view = service.request_export(&tenant, &exporter, form.row.id, None).await.unwrap();
        storage.fail_writes(true);
        FormsService::run_export(&service, &tenant, view.row.id).await.unwrap();
        let failed = service.get_export(&tenant, &exporter, view.row.id).await.unwrap();
        assert_eq!((failed.state, failed.download.is_none()), (ExportState::Failed, true));
        assert!(failed.row.expires_at.is_some(), "the daily job cleans failed exports too");
        storage.fail_writes(false);
        service.request_export(&tenant, &exporter, form.row.id, None).await.unwrap();
    })
    .await
    .unwrap();
}

/// File xuất chứa dữ liệu cá nhân: xoá theo email, xoá form và dọn theo hạn đều xoá file và dòng.
#[tokio::test]
async fn xoa_theo_email_xoa_form_va_don_dep_xoa_file_xuat() {
    TestDatabase::run(async |test| {
        let (service, database, storage) = exporting(test).await;
        let tenant = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let exporter = user(&["export"]);
        let mut ids = Vec::new();
        for key in ["contact", "survey", "old"] {
            let form = service.create_form(&tenant, &admin(), contact(key)).await.unwrap();
            send(&tenant, &service, key, &visitor("203.0.113.9"), answers("An", "an@example.com"))
                .await
                .unwrap();
            let view = service.request_export(&tenant, &exporter, form.row.id, None).await.unwrap();
            FormsService::run_export(&service, &tenant, view.row.id).await.unwrap();
            assert!(storage.bytes(&storage_key(view.row.id)).is_some());
            ids.push((form, view.row.id));
        }
        let gone = async |id| {
            storage.bytes(&storage_key(id)).is_none()
                && service.get_export(&tenant, &exporter, id).await.unwrap_err().code()
                    == "forms.export_not_found"
        };

        // Xoá theo email: mọi form có bài của địa chỉ đó.
        let (contact_form, contact_export) = &ids[0];
        let survey = service.create_form(&tenant, &admin(), contact("survey-2")).await.unwrap();
        send(
            &tenant,
            &service,
            "survey-2",
            &visitor("203.0.113.9"),
            answers("Bình", "binh@example.com"),
        )
        .await
        .unwrap();
        let untouched =
            service.request_export(&tenant, &exporter, survey.row.id, None).await.unwrap();
        FormsService::run_export(&service, &tenant, untouched.row.id).await.unwrap();
        service.erase_email(&tenant, &exporter, "an@example.com").await.unwrap();
        assert!(gone(*contact_export).await);
        assert!(gone(ids[1].1).await);
        assert!(!gone(untouched.row.id).await, "a form without the address keeps its export");
        let _ = contact_form;

        // Xoá form.
        service.delete_form(&tenant, &admin(), survey.row.id, None).await.unwrap();
        assert!(gone(untouched.row.id).await);

        // Dọn theo hạn (mặc định 24 giờ).
        let form = service.create_form(&tenant, &admin(), contact("later")).await.unwrap();
        let view = service.request_export(&tenant, &exporter, form.row.id, None).await.unwrap();
        FormsService::run_export(&service, &tenant, view.row.id).await.unwrap();
        let cleaned = service
            .cleanup(
                &autoxide_kernel::tenancy::DatabaseRegistry::single(database.clone()),
                &database,
                now() + chrono::Duration::hours(23),
            )
            .await
            .unwrap();
        assert_eq!(cleaned.exports, 0);
        let cleaned = service
            .cleanup(
                &autoxide_kernel::tenancy::DatabaseRegistry::single(database.clone()),
                &database,
                now() + chrono::Duration::hours(25),
            )
            .await
            .unwrap();
        assert!(cleaned.exports >= 1, "{cleaned:?}");
        assert!(gone(view.row.id).await);
    })
    .await
    .unwrap();
}

/// Bản xuất của một khách khác khách mặc định: file nằm dưới tiền tố của khách đó; link ký cho
/// khách này không mở được ở khách mặc định (khoá ký riêng mỗi khách); khách mặc định không thấy
/// bản xuất của nó.
#[tokio::test]
async fn ban_xuat_cua_khach_khac_nam_rieng() {
    TestDatabase::run(async |test| {
        let (service, database, storage) = exporting(test).await;
        let default = autoxide_kernel::tenancy::TenantContext::default_on(database.clone());
        let other = autoxide_kernel::tenancy::TenantContext::new(
            autoxide_kernel::tenancy::TenantId::from_uuid(Uuid::now_v7()),
            "other",
            "Other",
            autoxide_kernel::tenancy::TenantStatus::Active,
            autoxide_kernel::tenancy::MAIN_PLACEMENT,
            database.clone(),
        );
        // Cùng `key` ở hai khách: không đụng nhau.
        service.create_form(&default, &admin(), contact("contact")).await.unwrap();
        let form = service.create_form(&other, &admin(), contact("contact")).await.unwrap();
        send(
            &other,
            &service,
            "contact",
            &visitor("203.0.113.9"),
            answers("Ada", "ada@example.com"),
        )
        .await
        .unwrap();
        let exporter = user(&["export"]);
        let view = service.request_export(&other, &exporter, form.row.id, None).await.unwrap();
        FormsService::run_export(&service, &other, view.row.id).await.unwrap();

        let prefixed = format!("tenants/{}/{}", other.id(), storage_key(view.row.id));
        assert!(storage.bytes(&prefixed).is_some(), "the file is under the tenant prefix");
        assert!(storage.bytes(&storage_key(view.row.id)).is_none());

        let ready = service.get_export(&other, &exporter, view.row.id).await.unwrap();
        let (expires, signature) = link_parts(&ready.download.unwrap().0);
        service.download_export(&other, view.row.id, expires, &signature).await.unwrap();
        let elsewhere = service.download_export(&default, view.row.id, expires, &signature).await;
        assert_eq!(
            elsewhere.err().map(|error| error.code().to_owned()).as_deref(),
            Some("forms.export_link_invalid")
        );
        let unseen = service.get_export(&default, &exporter, view.row.id).await.unwrap_err();
        assert_eq!(unseen.code(), "forms.export_not_found");
        let unseen = service.get_form(&default, &admin(), form.row.id).await.unwrap_err();
        assert_eq!(unseen.code(), "forms.not_found");
    })
    .await
    .unwrap();
}

//! Mã đăng nhập bằng số (bước 47): `compliance` cắm bộ mã vào `auth` qua `AuthExtensions`; mã dùng
//! một lần, sai thì đếm, hai lần nhập cùng lúc chỉ một bên được; gắn số vào tài khoản theo quy tắc số
//! tài khoản mỗi số. Cả SQLite và Postgres.

use std::sync::{Arc, Mutex};

use autoxide_auth::api::{AuthExtensions, PhoneCodes};
use autoxide_kernel::error::Error;
use autoxide_kernel::sms::SmsSender;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, ScopeExt};
use autoxide_testkit::{FakeSmsSender, TestApp, TestDatabase};
use uuid::Uuid;

use crate::api::AccountVerification;

/// `auth` giả: giữ bộ mã `compliance` cắm vào.
#[derive(Default)]
struct Capture(Mutex<Option<Arc<dyn PhoneCodes>>>);

impl AuthExtensions for Capture {
    fn set_phone_codes(&self, codes: Arc<dyn PhoneCodes>) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(codes);
        Ok(())
    }
}

async fn app(test: &TestDatabase) -> (TestApp, Arc<dyn PhoneCodes>, Arc<FakeSmsSender>) {
    app_with(test, &[]).await
}

async fn app_with(
    test: &TestDatabase,
    settings: &[(&str, &str)],
) -> (TestApp, Arc<dyn PhoneCodes>, Arc<FakeSmsSender>) {
    let sms = Arc::new(FakeSmsSender::new());
    let capture = Arc::new(Capture::default());
    let sender: Arc<dyn SmsSender> = sms.clone();
    // `alice`: người đã đăng nhập, xác thực lại số để mở khoá (D-20261126-otp-nist).
    let mut builder = TestApp::builder(test).user("alice", 2, &[]);
    for (name, value) in settings {
        builder = builder.setting(name, value);
    }
    let app = builder
        .provide::<dyn SmsSender>(sender)
        .provide::<dyn AuthExtensions>(capture.clone())
        .module(crate::module())
        .build()
        .await
        .unwrap();
    let codes = capture.0.lock().unwrap().clone().expect("compliance plugged its codes in");
    (app, codes, sms)
}

fn code_in(sms: &FakeSmsSender, phone: &str) -> String {
    let text = sms.last_text(phone).unwrap();
    text.split(|c: char| !c.is_ascii_digit()).find(|part| part.len() == 6).unwrap().to_owned()
}

#[tokio::test]
async fn ma_dang_nhap_dung_mot_lan_va_gan_so_theo_quy_tac() {
    TestDatabase::run(async |test| {
        let (app, codes, sms) = app(test).await;
        let tenant = app.default_tenant();
        let sent = codes.send(&tenant, "0912 345 678", None).await.unwrap();
        let code = code_in(&sms, "+84912345678");
        let wrong = if code == "000000" { "111111" } else { "000000" };
        let error = codes.verify(&tenant, sent.id, wrong).await.unwrap_err();
        assert_eq!(error.code(), "compliance.otp_invalid");
        let phone = codes.verify(&tenant, sent.id, &code).await.unwrap();
        assert_eq!(phone.as_str(), "+84912345678");
        // Dùng một lần.
        let error = codes.verify(&tenant, sent.id, &code).await.unwrap_err();
        assert_eq!(error.code(), "compliance.otp_invalid");
        // Mã lạ.
        let error = codes.verify(&tenant, Uuid::now_v7(), &code).await.unwrap_err();
        assert_eq!(error.code(), "compliance.challenge_not_found");

        // Gắn số vào tài khoản: module khác thấy qua `AccountVerification`.
        let accounts = app.api::<dyn AccountVerification>().unwrap();
        let (lan, minh) = (Uuid::from_u128(21), Uuid::from_u128(22));
        codes.record(&tenant, lan, &phone).await.unwrap();
        let verified = accounts.verified_phone(DEFAULT_TENANT, lan).await.unwrap();
        assert_eq!(
            verified.map(|phone| phone.as_str().to_owned()).as_deref(),
            Some("+84912345678")
        );
        // Quy tắc mặc định một tài khoản mỗi số: tài khoản thứ hai không được gắn, không lỗi.
        codes.record(&tenant, minh, &phone).await.unwrap();
        assert!(accounts.verified_phone(DEFAULT_TENANT, minh).await.unwrap().is_none());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hai_lan_nhap_ma_cung_luc_chi_mot_ben_duoc() {
    TestDatabase::run(async |test| {
        let (app, codes, sms) = app(test).await;
        let tenant = app.default_tenant();
        let sent = codes.send(&tenant, "0987654321", None).await.unwrap();
        let code = code_in(&sms, "+84987654321");
        let (left, right) = tokio::join!(
            codes.verify(&tenant, sent.id, &code),
            codes.verify(&tenant, sent.id, &code),
        );
        assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1, "{left:?} {right:?}");
    })
    .await
    .unwrap();
}

/// Hạn mức sai mỗi ngày của một số: cộng mọi mã của số trong 24 giờ — xin mã mới không mở thêm
/// lượt đoán; đủ hạn mức thì không kiểm, không gửi mã nữa, tới khi lần sai cũ ra khỏi 24 giờ.
#[tokio::test]
async fn sai_qua_han_muc_moi_ngay_thi_khoa_so() {
    TestDatabase::run(async |test| {
        let settings = [
            ("AUTOXIDE_COMPLIANCE_OTP_WRONG_PER_PHONE_PER_DAY", "4"),
            ("AUTOXIDE_COMPLIANCE_OTP_PER_PHONE_PER_HOUR", "100"),
        ];
        let (app, codes, sms) = app_with(test, &settings).await;
        let tenant = app.default_tenant();
        let wrong_for = |code: &str| if code == "000000" { "111111" } else { "000000" }.to_owned();

        let first = codes.send(&tenant, "0912 000 111", None).await.unwrap();
        let wrong = wrong_for(&code_in(&sms, "+84912000111"));
        for _ in 0..3 {
            let error = codes.verify(&tenant, first.id, &wrong).await.unwrap_err();
            assert_eq!(error.code(), "compliance.otp_invalid");
        }
        // Mã mới: chỉ còn một lượt sai trong ngày.
        let second = codes.send(&tenant, "0912000111", None).await.unwrap();
        let code = code_in(&sms, "+84912000111");
        let error = codes.verify(&tenant, second.id, &wrong_for(&code)).await.unwrap_err();
        assert_eq!(error.code(), "compliance.otp_invalid");
        // Đủ 4 lần sai: mã đúng cũng không được kiểm, không gửi thêm mã.
        let error = codes.verify(&tenant, second.id, &code).await.unwrap_err();
        assert_eq!(error.code(), "compliance.otp_daily_limit");
        assert!(error.retry_after().is_some_and(|seconds| seconds > 0));
        let sent_before = sms.sent.lock().unwrap().len();
        let error = codes.send(&tenant, "0912000111", None).await.unwrap_err();
        assert_eq!(error.code(), "compliance.otp_daily_limit");
        assert_eq!(
            sms.sent.lock().unwrap().len(),
            sent_before,
            "không gửi SMS khi số đang bị khoá"
        );
        // Số khác không bị ảnh hưởng.
        codes.send(&tenant, "0912000222", None).await.unwrap();

        // Qua 24 giờ (dời mốc tạo của các mã): gửi và kiểm lại được.
        {
            use sea_orm::sea_query::Expr;
            use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
            let past =
                autoxide_kernel::time::Timestamp::now() - jiff::SignedDuration::from_hours(25);
            crate::entity::phone_challenge::Entity::update_many()
                .scoped(&tenant)
                .col_expr(crate::entity::phone_challenge::Column::CreatedAt, Expr::value(past))
                .filter(crate::entity::phone_challenge::Column::Phone.eq("+84912000111"))
                .exec(app.database().write())
                .await
                .unwrap();
        }
        let third = codes.send(&tenant, "0912000111", None).await.unwrap();
        let code = code_in(&sms, "+84912000111");
        codes.verify(&tenant, third.id, &code).await.unwrap();
    })
    .await
    .unwrap();
}

fn ip(text: &str) -> Option<std::net::IpAddr> {
    Some(text.parse().unwrap())
}

/// PT-4: giới hạn gửi mã theo địa chỉ đếm theo khối /64 với IPv6, theo đúng địa chỉ với IPv4 — xoay địa chỉ trong khối của
/// mình không gửi thêm được mã tới hàng loạt số (mỗi tin tốn tiền SMS của cửa hàng).
#[tokio::test]
async fn xoay_dia_chi_trong_khoi_khong_gui_them_ma() {
    TestDatabase::run(async |test| {
        let settings = [("AUTOXIDE_COMPLIANCE_OTP_PER_IP_PER_HOUR", "5")];
        let (app, codes, _sms) = app_with(test, &settings).await;
        let tenant = app.default_tenant();
        for number in 0..5u32 {
            let client = ip(&format!("2001:db8:5:2::{:x}", number + 1));
            codes.send(&tenant, &format!("09120{number:05}"), client).await.unwrap();
        }
        let error = codes.send(&tenant, "0912000099", ip("2001:db8:5:2::abcd")).await.unwrap_err();
        assert_eq!(error.code(), "compliance.otp_throttled");
        // IPv4: theo đúng địa chỉ; địa chỉ bên cạnh trong cùng /24 (NAT của nhà mạng) không bị chặn oan.
        for number in 0..5u32 {
            codes.send(&tenant, &format!("09130{number:05}"), ip("198.51.100.7")).await.unwrap();
        }
        let error = codes.send(&tenant, "0913000099", ip("198.51.100.7")).await.unwrap_err();
        assert_eq!(error.code(), "compliance.otp_throttled");
        codes.send(&tenant, "0913000098", ip("198.51.100.8")).await.unwrap();
        // Khối khác vẫn gửi được.
        codes.send(&tenant, "0914000001", ip("2001:db8:5:3::1")).await.unwrap();
    })
    .await
    .unwrap();
}

/// PT-7 (D-20261126-otp-nist): người lạ xin mã cho số của người khác rồi nhập sai cho đủ hạn mức ngày không khoá được
/// chủ số ở địa chỉ khác; chính khối của họ thì bị khoá. Lần đúng xoá các lần sai trước (NIST SP 800-63B-4 §3.2.2). Đoán
/// sai từ nhiều khối chạm trần chung → số bị khoá với mọi client lạ, nhưng client đã đăng nhập đúng bằng số (client quen)
/// vẫn xin và dùng mã được.
#[tokio::test]
async fn doan_sai_tu_khoi_khac_khong_khoa_chu_so() {
    TestDatabase::run(async |test| {
        let settings = [
            ("AUTOXIDE_COMPLIANCE_OTP_WRONG_PER_PHONE_PER_DAY", "2"),
            ("AUTOXIDE_COMPLIANCE_OTP_WRONG_PER_PHONE_ALL_CLIENTS_PER_DAY", "4"),
            ("AUTOXIDE_COMPLIANCE_OTP_PER_PHONE_PER_HOUR", "100"),
        ];
        let (app, codes, sms) = app_with(test, &settings).await;
        let tenant = app.default_tenant();
        let wrong_for = |code: &str| if code == "000000" { "111111" } else { "000000" }.to_owned();
        let burn = async |client: &str| {
            let sent = codes.send(&tenant, "0912000333", ip(client)).await.unwrap();
            let wrong = wrong_for(&code_in(&sms, "+84912000333"));
            for _ in 0..2 {
                let error = codes.verify(&tenant, sent.id, &wrong).await.unwrap_err();
                assert_eq!(error.code(), "compliance.otp_invalid");
            }
        };
        burn("203.0.113.10").await;
        // Khối của kẻ nhập sai: bị khoá.
        let error = codes.send(&tenant, "0912000333", ip("203.0.113.10")).await.unwrap_err();
        assert_eq!(error.code(), "compliance.otp_daily_limit");
        // Chủ số ở khối khác: vẫn xin và dùng mã được; khối của họ thành client quen.
        let own = codes.send(&tenant, "0912000333", ip("2001:db8:9:1::1")).await.unwrap();
        codes.verify(&tenant, own.id, &code_in(&sms, "+84912000333")).await.unwrap();

        // Sau lần đúng: 2 khối × 2 lần sai = 4 = trần chung → client lạ bị khoá ở mọi nơi.
        for block in 1..3 {
            burn(&format!("198.51.{block}.1")).await;
        }
        let error = codes.send(&tenant, "0912000333", ip("2001:db8:9:2::1")).await.unwrap_err();
        assert_eq!(error.code(), "compliance.otp_daily_limit");
        // Client quen của chủ số (cùng khối /64): không bị trần chung chặn.
        let again = codes.send(&tenant, "0912000333", ip("2001:db8:9:1::2")).await.unwrap();
        codes.verify(&tenant, again.id, &code_in(&sms, "+84912000333")).await.unwrap();
    })
    .await
    .unwrap();
}

/// NIST SP 800-63B-4 §3.1.3.2: mã chỉ có hiệu lực tối đa 10 phút — cấu hình dài hơn bị từ chối khi khởi động.
#[test]
fn ma_khong_song_qua_muoi_phut() {
    use autoxide_kernel::config::{ConfigReader, ConfigSource};
    let read = |ttl: &str| {
        let source = ConfigSource::from_pairs([("AUTOXIDE_COMPLIANCE_OTP_TTL_SECONDS", ttl)]);
        crate::config::read(&mut ConfigReader::new(&source)).map(|config| config.otp.ttl_seconds)
    };
    assert_eq!(read("600").ok(), Some(600));
    assert!(read("601").is_err());
}

/// Số mã mỗi giờ theo cặp (số, client) — người lạ tiêu hết lượt của họ không làm chủ số hết lượt; trần chung của số
/// chặn người lạ từ nhiều nơi nhưng không chặn client quen của chủ số.
#[tokio::test]
async fn nguoi_la_het_luot_gui_ma_chu_so_van_nhan_duoc() {
    TestDatabase::run(async |test| {
        let settings = [
            ("AUTOXIDE_COMPLIANCE_OTP_PER_PHONE_PER_HOUR", "2"),
            ("AUTOXIDE_COMPLIANCE_OTP_PER_PHONE_ALL_CLIENTS_PER_HOUR", "3"),
        ];
        let (app, codes, sms) = app_with(test, &settings).await;
        let tenant = app.default_tenant();
        // Chủ số đăng nhập đúng từ mạng của họ: thành client quen.
        let own = codes.send(&tenant, "0912000444", ip("2001:db8:7:1::1")).await.unwrap();
        codes.verify(&tenant, own.id, &code_in(&sms, "+84912000444")).await.unwrap();
        // Người lạ ở X: 2 mã rồi hết lượt của cặp (số, X).
        for _ in 0..2 {
            codes.send(&tenant, "0912000444", ip("203.0.113.20")).await.unwrap();
        }
        let error = codes.send(&tenant, "0912000444", ip("203.0.113.20")).await.unwrap_err();
        assert_eq!(error.code(), "compliance.otp_throttled");
        // Người lạ ở Y: trần chung của số (3: chủ số lúc còn lạ + 2 của X) đã đủ.
        let error = codes.send(&tenant, "0912000444", ip("198.51.100.30")).await.unwrap_err();
        assert_eq!(error.code(), "compliance.otp_throttled");
        // Chủ số ở mạng quen: vẫn nhận mã và đăng nhập được.
        let again = codes.send(&tenant, "0912000444", ip("2001:db8:7:1::2")).await.unwrap();
        codes.verify(&tenant, again.id, &code_in(&sms, "+84912000444")).await.unwrap();
    })
    .await
    .unwrap();
}

/// NIST SP 800-63B-4 §3.2.2: 100 lần sai liên tiếp (mọi client, không theo cửa sổ thời gian) khoá mã đăng nhập của số
/// với mọi client, kể cả client quen. Đăng nhập đúng cho hết hạn mọi mã còn mở khác của số. Chủ số xác thực lại số từ
/// tài khoản đã đăng nhập thì mở khoá.
#[tokio::test]
async fn sai_lien_tiep_toi_tran_cung_thi_khoa_den_khi_xac_thuc_lai() {
    TestDatabase::run(async |test| {
        use sea_orm::EntityTrait;
        use sea_orm::sea_query::Expr;
        let settings = [("AUTOXIDE_COMPLIANCE_OTP_PER_PHONE_PER_HOUR", "100")];
        let (app, codes, sms) = app_with(test, &settings).await;
        let tenant = app.default_tenant();
        let wrong_for = |code: &str| if code == "000000" { "111111" } else { "000000" }.to_owned();
        let home = ip("2001:db8:8:1::1");

        // Mã của người lạ tạo trước lần đúng của chủ số: hết hạn khi chủ số đăng nhập đúng.
        let stale = codes.send(&tenant, "0912000555", ip("203.0.113.40")).await.unwrap();
        let stale_code = code_in(&sms, "+84912000555");
        let own = codes.send(&tenant, "0912000555", home).await.unwrap();
        codes.verify(&tenant, own.id, &code_in(&sms, "+84912000555")).await.unwrap();
        let error = codes.verify(&tenant, stale.id, &stale_code).await.unwrap_err();
        assert_eq!(
            error.code(),
            "compliance.otp_invalid",
            "codes asked before the right one are closed"
        );

        // Một lần sai thật, rồi đưa bộ đếm tới 99 (như sau nhiều ngày đoán từ nhiều nơi), rồi một lần sai nữa.
        let first = codes.send(&tenant, "0912000555", ip("203.0.113.41")).await.unwrap();
        let wrong = wrong_for(&code_in(&sms, "+84912000555"));
        codes.verify(&tenant, first.id, &wrong).await.unwrap_err();
        crate::entity::phone_failure::Entity::update_many()
            .col_expr(crate::entity::phone_failure::Column::Failures, Expr::value(99))
            .exec(app.database().write())
            .await
            .unwrap();
        let error = codes.verify(&tenant, first.id, &wrong).await.unwrap_err();
        assert_eq!(error.code(), "compliance.otp_invalid");
        // Đủ 100: khoá với mọi client, kể cả mạng quen của chủ số.
        let error = codes.send(&tenant, "0912000555", home).await.unwrap_err();
        assert_eq!(error.code(), "compliance.otp_locked");

        // Mở khoá: chủ số đăng nhập cách khác (`alice`) và xác thực lại số trong tài khoản.
        let (status, sent) = crate::tests::call(
            &app,
            "POST",
            "/me/phone-verifications",
            "alice",
            Some(serde_json::json!({"phone": "0912000555"})),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::CREATED, "{sent}");
        let id = sent["data"]["id"].as_str().unwrap();
        let body = serde_json::json!({"code": code_in(&sms, "+84912000555")});
        let uri = format!("/me/phone-verifications/{id}/confirm");
        let (status, confirmed) = crate::tests::call(&app, "POST", &uri, "alice", Some(body)).await;
        assert_eq!(status, axum::http::StatusCode::OK, "{confirmed}");
        let again = codes.send(&tenant, "0912000555", home).await.unwrap();
        codes.verify(&tenant, again.id, &code_in(&sms, "+84912000555")).await.unwrap();
    })
    .await
    .unwrap();
}

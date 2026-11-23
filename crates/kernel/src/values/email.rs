//! Email addresses submitted by outsiders (forms, comments, newsletter sign-ups, ...): normalization and shape
//! checking, one way for every module (D-174).
//!
//! The rules are sufficient rather than the full RFC 5322: enough to block junk text and to be sure mail can
//! be delivered. Length is counted in BYTES like the mail sender (`lettre`) and RFC 5321: the whole address
//! is at most [`MAX_LENGTH`] bytes, the part before `@` is 1-64 bytes, the domain has at least one dot, and
//! each label is 1-63 bytes of letters, digits or `-` (not at the start or end of a label).

use crate::error::FieldError;

/// Maximum length of an address in bytes (RFC 5321: a 256-byte path including `<` and `>`).
pub const MAX_LENGTH: usize = 254;
const LOCAL_MAX: usize = 64;
const LABEL_MAX: usize = 63;
/// Characters not allowed in the part before `@` (special characters of mail headers).
const LOCAL_FORBIDDEN: &str = "<>()[]\\,;:\"@";

/// Email for storing and lookup: surrounding whitespace removed, lowercased.
pub fn normalize(email: &str) -> String {
    email.trim().to_lowercase()
}

/// Whether `email` has the right shape `local-part@domain` under the rules at the top of the module. Call it on
/// a value already passed through [`normalize`].
pub fn is_valid(email: &str) -> bool {
    if email.len() > MAX_LENGTH {
        return false;
    }
    let Some((local, domain)) = email.split_once('@') else { return false };
    let local_ok = (1..=LOCAL_MAX).contains(&local.len())
        && !local.starts_with('.')
        && !local.ends_with('.')
        && !local.contains("..")
        && local
            .chars()
            .all(|c| !c.is_whitespace() && !c.is_control() && !LOCAL_FORBIDDEN.contains(c));
    let labels: Vec<&str> = domain.split('.').collect();
    let domain_ok = labels.len() >= 2
        && labels.iter().all(|label| {
            (1..=LABEL_MAX).contains(&label.len())
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_alphanumeric() || c == '-')
        });
    local_ok && domain_ok
}

/// Error of field `field` when the email has the wrong shape (code `email`).
pub fn invalid(field: &str) -> FieldError {
    FieldError::new(field, "email", "must be an email address like `name@example.com`")
}

/// The normalized email of `raw`; if the shape is wrong, adds an error of `field` to `problems` and returns
/// `None` (the service gathers all errors and reports them once, D-083).
pub fn checked(raw: &str, field: &str, problems: &mut Vec<FieldError>) -> Option<String> {
    let email = normalize(raw);
    if is_valid(&email) {
        Some(email)
    } else {
        problems.push(invalid(field));
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dia_chi_hop_le() {
        for email in [
            "ada@example.com",
            "a.b+tag@mail.example.co.uk",
            "x@xn--bcher-kva.example",
            "người.dùng@ví-dụ.vn",
            "o'brien@example.org",
        ] {
            assert!(is_valid(email), "{email}");
        }
    }

    #[test]
    fn dia_chi_sai_dang() {
        for email in [
            "",
            "ada",
            "ada@",
            "@example.com",
            "ada@localhost",
            "ada@@example.com",
            ".ada@example.com",
            "ada.@example.com",
            "a..da@example.com",
            "a da@example.com",
            "a<da@example.com",
            "ada@-example.com",
            "ada@example-.com",
            "ada@exa_mple.com",
            "ada@example..com",
            "ada@example.com.",
        ] {
            assert!(!is_valid(email), "{email:?}");
        }
    }

    #[test]
    fn do_dai_dem_theo_byte() {
        let local = "a".repeat(64);
        assert!(is_valid(&format!("{local}@example.com")));
        assert!(!is_valid(&format!("{local}a@example.com")));
        // 32 chữ `đ` là 32 ký tự nhưng 64 byte: vừa đủ; thêm một chữ thì quá.
        assert!(is_valid(&format!("{}@example.com", "đ".repeat(32))));
        assert!(!is_valid(&format!("{}@example.com", "đ".repeat(33))));
        let label = "b".repeat(63);
        assert!(is_valid(&format!("a@{label}.com")));
        assert!(!is_valid(&format!("a@{label}b.com")));
        // Tổng 254 byte thì nhận, 255 thì không.
        let domain =
            format!("{}.{}.{}.{}", "c".repeat(63), "c".repeat(63), "c".repeat(63), "c".repeat(59));
        let at_limit = format!("ab@{domain}");
        assert_eq!(at_limit.len(), 254);
        assert!(is_valid(&at_limit));
        assert!(!is_valid(&format!("abc@{domain}")));
    }

    #[test]
    fn chuan_hoa_roi_kiem() {
        assert_eq!(normalize("  Ada@Example.COM \n"), "ada@example.com");
        let mut problems = Vec::new();
        assert_eq!(
            checked(" Ada@Example.com ", "email", &mut problems),
            Some("ada@example.com".to_owned())
        );
        assert!(problems.is_empty());
        assert_eq!(checked("nope", "reply_to", &mut problems), None);
        assert_eq!(problems.len(), 1);
        assert_eq!((problems[0].field.as_str(), problems[0].code.as_str()), ("reply_to", "email"));
    }

    // Property test (đặc tả 31 B3): chuỗi bất kỳ, kể cả Unicode lạ; độ dài đếm theo byte (D-174).
    proptest::proptest! {
        #[test]
        fn email_bat_bien(raw in "\\PC{0,80}|[a-zA-Z0-9._%+-]{1,70}@[a-z0-9-]{1,70}(\\.[a-z0-9-]{1,70}){1,3}") {
            let email = normalize(&raw);
            proptest::prop_assert_eq!(normalize(&email), email.clone(), "normalize is idempotent");
            if is_valid(&email) {
                proptest::prop_assert!(email.len() <= MAX_LENGTH);
                let (local, domain) = email.split_once('@').unwrap();
                proptest::prop_assert!((1..=LOCAL_MAX).contains(&local.len()));
                proptest::prop_assert!(domain.contains('.'));
                proptest::prop_assert!(domain.split('.').all(|label| (1..=LABEL_MAX).contains(&label.len())));
            }
            let mut problems = Vec::new();
            proptest::prop_assert_eq!(checked(&raw, "email", &mut problems).is_some(), is_valid(&email));
            proptest::prop_assert_eq!(problems.is_empty(), is_valid(&email));
        }
    }
}

//! Sinh nội dung file `.env.example` từ danh sách biến đã khai trong code.

use super::SettingValue;

const HEADER: &str = "\
# Autoxide configuration.
#
# Copy this file to `.env` and uncomment the lines you want to change. Real
# environment variables take precedence over `.env`. Never commit `.env`.
#
# This file is generated from the settings declared in the code: run
# `autoxide config --env-example`. A test fails when it is out of date.
";

/// Độ rộng tối đa của một dòng chú thích.
const WIDTH: usize = 80;

/// Nội dung `.env.example` cho các biến trong `values` (theo thứ tự đã đọc).
pub fn render_env_example(values: &[SettingValue]) -> String {
    let mut output = String::from(HEADER);
    for value in values {
        let setting = &value.setting;
        output.push('\n');
        for line in wrap(setting.description, WIDTH - 2) {
            output.push_str("# ");
            output.push_str(&line);
            output.push('\n');
        }
        match (setting.default, value.optional) {
            (Some(default), _) => {
                output.push_str(&format!("# Default: {default}\n#{}={default}\n", setting.name));
            }
            (None, true) => {
                let example = setting.example.unwrap_or_default();
                output.push_str(&format!("# Optional.\n#{}={example}\n", setting.name));
            }
            // Bắt buộc: để dòng KHÔNG bị chú thích, người cài đặt thấy ngay phải điền.
            (None, false) => {
                let example = setting.example.unwrap_or_default();
                output.push_str(&format!("# Required.\n{}={example}\n", setting.name));
            }
        }
    }
    output
}

/// Ngắt `text` thành các dòng dài tối đa `width` ký tự, ngắt ở khoảng trắng.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match lines.last_mut() {
            Some(line) if line.chars().count() + 1 + word.chars().count() <= width => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_owned()),
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ConfigReader, ConfigSource, Setting};

    const WITH_DEFAULT: Setting = Setting::new("PORT", "Port to listen on.").with_default("8080");
    const REQUIRED: Setting =
        Setting::new("SECRET_KEY", "Key used to sign tokens.").secret().with_example("change-me");
    const OPTIONAL: Setting = Setting::new("LIMIT", "Optional limit.");

    #[test]
    fn moi_loai_bien_co_mot_dang_dong() {
        let source = ConfigSource::from_pairs([("SECRET_KEY", "real-secret"), ("PORT", "9999")]);
        let mut reader = ConfigReader::new(&source);
        let _ = reader.get::<u16>(&WITH_DEFAULT);
        let _ = reader.get::<String>(&REQUIRED);
        let _ = reader.optional::<u32>(&OPTIONAL);

        let rendered = render_env_example(reader.values());

        let body = rendered.strip_prefix(HEADER).unwrap();
        assert_eq!(
            body,
            "\n# Port to listen on.\n# Default: 8080\n#PORT=8080\n\
             \n# Key used to sign tokens.\n# Required.\nSECRET_KEY=change-me\n\
             \n# Optional limit.\n# Optional.\n#LIMIT=\n"
        );
        // File mẫu chỉ chứa mặc định và ví dụ, không bao giờ chứa giá trị đang đặt.
        assert!(!rendered.contains("real-secret"));
        assert!(!rendered.contains("9999"));
    }

    #[test]
    fn mo_ta_dai_duoc_ngat_dong() {
        let lines = wrap("one two three four five six seven", 13);
        assert_eq!(lines, ["one two three", "four five six", "seven"]);
        assert!(wrap("", 10).is_empty());
        // Một từ dài hơn độ rộng thì đứng riêng một dòng, không bị cắt.
        assert_eq!(wrap("a verylongword b", 5), ["a", "verylongword", "b"]);
    }
}

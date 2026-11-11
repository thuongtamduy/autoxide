//! Passkey (WebAuthn): đăng nhập bằng vân tay, Face ID, khoá bảo mật thay cho mật khẩu
//! (D-077).

mod entity;
mod service;
mod webauthn;

pub(crate) use entity::passkey::Model as Passkey;
pub(crate) use service::{Ceremony, PasskeyService};

#[cfg(test)]
pub(crate) mod testing;
#[cfg(test)]
mod tests;

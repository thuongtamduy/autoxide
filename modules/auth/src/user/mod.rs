//! Tài khoản người dùng: bảng `auth_user` và service làm việc với nó.

mod entity;
mod service;

pub(crate) use service::UserService;

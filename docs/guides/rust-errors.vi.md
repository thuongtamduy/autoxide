# Lỗi biên dịch Rust hay gặp khi viết module

Bảy lỗi mà gần như ai cũng gặp trong tuần đầu, kèm dòng cần tìm, vì sao nó xảy ra trong một module Autoxide
và cách sửa. Thông báo là kết quả thật của toolchain đã ghim. Lỗi không có ở đây thì chạy
`rustc --explain E0xxx` (mã ở dòng đầu) và đọc các dòng "help:": chúng thường đúng.

## 1. `no field … on type impl Future` (E0609)

```text
error[E0609]: no field `title` on type `impl Future<Output = Result<Note, String>>`
```

Gọi một `async fn` trả về một future, không phải giá trị. Thêm `.await`: `let note = find(1).await?;`. Service,
lệnh database và `Outbox::record` đều là `async`.

## 2. `` `?` couldn't convert the error to `Error` `` (E0277)

```text
error[E0277]: `?` couldn't convert the error to `Error`
  the trait `From<ParseIntError>` is not implemented for `Error`
```

`?` đổi lỗi bên phải thành kiểu lỗi của hàm bằng `From`. Trong module, hàm trả `autoxide_kernel::error::Error`,
kiểu này đổi được từ lỗi database và lỗi kiểm đầu vào. Lỗi khác thì tự đổi và gắn một `code` ổn định:
`.map_err(|_| Error::bad_request("notes.invalid_number", "…"))?`.

## 3. `borrow of moved value` (E0382)

```text
error[E0382]: borrow of moved value: `title`
  value moved here  /  value borrowed here after move
```

Truyền một `String` vào hàm là đưa nó đi. Hoặc truyền tham chiếu (`&title`, và hàm nhận `&str`), hoặc `.clone()`
khi thật sự cần hai chủ. Nên chọn tham chiếu: chính gợi ý của trình biên dịch nói "consider changing this
parameter type … to borrow instead".

## 4. `future cannot be sent between threads safely`

```text
error: future cannot be sent between threads safely
  the trait `Send` is not implemented for `Rc<i32>`
  future is not `Send` as this value is used across an await
```

Handler và service chạy trên nhiều thread, nên mọi thứ còn sống qua một `.await` phải là `Send`. `Rc` và `RefCell`
thì không (dùng `Arc` và một lock); `std::sync::MutexGuard` cũng không: cho nó hết phạm vi (hoặc `drop`) trước khi
`.await`. Không giữ guard của transaction database cùng một lock qua một lệnh gọi chậm.

## 5. `async block may outlive the current function` (E0373)

```text
error[E0373]: async block may outlive the current function, but it borrows `name`
help: to force the async block to take ownership of `name` … use the `move` keyword
```

Task được spawn (`tokio::spawn`, handler của job) có thể chạy sau khi hàm trả về, nên nó phải sở hữu thứ nó dùng.
Viết `async move { … }` và clone trước khối nếu phần sau còn cần.

## 6. `non-exhaustive patterns` (E0004)

```text
error[E0004]: non-exhaustive patterns: `Status::Published` not covered
```

`match` phải xử lý mọi variant. Thêm nhánh còn thiếu. Với enum của kernel đánh dấu `#[non_exhaustive]`, trình biên dịch
còn đòi nhánh cuối `_ =>`, có chủ đích: kernel có thể thêm variant ở bản minor và code của bạn không được vỡ khi đó.

## 7. `no method named … found` dù method có tồn tại (E0599)

```text
error[E0599]: no method named `find` found for struct `S` in the current scope
  items from traits can only be used if the trait is in scope
help: … perhaps you want to import it: use crate::api::Notes;
```

Method của trait cần import trait. Với service của module khác thì `use autoxide_<tên>::api::<Trait>;`. Cũng vậy với
`ScopeExt` (`.scoped(&tenant)`) và các trait của `sea_orm` (`EntityTrait`, `QueryFilter`) trong API dựa trên SeaORM hiện
tại.

## Lỗi từ các bước kiểm của repo

`cargo xtask check` và `cargo xtask check-module` in `how to fix:` dưới mỗi vi phạm. Bắt đầu từ đó.

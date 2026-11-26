# Common Rust compiler errors when writing a module

Seven errors that almost everyone meets in the first week, with the line to look for, why it happens in an
Autoxide module, and the fix. The messages are real output of the pinned toolchain. When the error is not
here, run `rustc --explain E0xxx` (the code in the first line) and read the "help:" lines: they are usually right.

## 1. `no field … on type impl Future` (E0609)

```text
error[E0609]: no field `title` on type `impl Future<Output = Result<Note, String>>`
```

A call to an `async fn` returns a future, not the value. Add `.await`: `let note = find(1).await?;`. Services,
database calls and `Outbox::record` are all `async`.

## 2. `` `?` couldn't convert the error to `Error` `` (E0277)

```text
error[E0277]: `?` couldn't convert the error to `Error`
  the trait `From<ParseIntError>` is not implemented for `Error`
```

`?` turns the error on the right into the error type of your function using `From`. In a module your
functions return `autoxide_kernel::error::Error`, which converts from the database error and from validation
errors. For anything else map it yourself and give it a stable `code`:
`.map_err(|_| Error::bad_request("notes.invalid_number", "…"))?`.

## 3. `borrow of moved value` (E0382)

```text
error[E0382]: borrow of moved value: `title`
  value moved here  /  value borrowed here after move
```

Passing a `String` to a function gives it away. Either pass a reference (`&title`, and take `&str` in the
function), or `.clone()` it when you really need two owners. Prefer the reference: the compiler's own hint says
"consider changing this parameter type … to borrow instead".

## 4. `future cannot be sent between threads safely`

```text
error: future cannot be sent between threads safely
  the trait `Send` is not implemented for `Rc<i32>`
  future is not `Send` as this value is used across an await
```

Handlers and services run on several threads, so everything alive across an `.await` must be `Send`. `Rc` and
`RefCell` are not (use `Arc` and a lock); a `std::sync::MutexGuard` is not either: end its scope (or `drop` it)
before the `.await`. Never hold a database transaction guard and a lock at the same time across a slow call.

## 5. `async block may outlive the current function` (E0373)

```text
error[E0373]: async block may outlive the current function, but it borrows `name`
help: to force the async block to take ownership of `name` … use the `move` keyword
```

A spawned task (`tokio::spawn`, a job handler) can run after the function returns, so it must own what it
uses. Write `async move { … }` and clone what you still need afterwards, before the block.

## 6. `non-exhaustive patterns` (E0004)

```text
error[E0004]: non-exhaustive patterns: `Status::Published` not covered
```

A `match` must handle every variant. Add the arm. For an enum of the kernel marked `#[non_exhaustive]` the
compiler also asks for a final `_ =>` arm, on purpose: the kernel may add variants in a minor release, and your
code must not break when it does.

## 7. `no method named … found` although the method exists (E0599)

```text
error[E0599]: no method named `find` found for struct `S` in the current scope
  items from traits can only be used if the trait is in scope
help: … perhaps you want to import it: use crate::api::Notes;
```

Trait methods need the trait imported. For another module's service that is `use autoxide_<name>::api::<Trait>;`.
Same for `ScopeExt` (`.scoped(&tenant)`) and the `sea_orm` traits (`EntityTrait`, `QueryFilter`) in the current
SeaORM-based API.

## Errors from the repository's own checks

`cargo xtask check` and `cargo xtask check-module` print `how to fix:` under every violation. Start there.

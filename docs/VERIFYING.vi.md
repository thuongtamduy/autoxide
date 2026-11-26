# Kiểm một bản phát hành

[English](VERIFYING.md) · **Tiếng Việt**

Mọi bản phát hành của Autoxide do workflow `Release` của repo này build và ký trên GitHub Actions. **Không có khoá ký
dài hạn nào**: [Sigstore](https://www.sigstore.dev/) cấp một chứng chỉ ngắn hạn gắn với danh tính của lần chạy
workflow (token OIDC của GitHub), và chữ ký được ghi vào log minh bạch công khai Rekor. Kiểm chữ ký cho bạn biết file
hay image do workflow phát hành của repo này build từ một tag phát hành, và chưa bị đổi từ đó. Nó không cho biết code
không có lỗi.

Thay `X.Y.Z` dưới đây bằng phiên bản bạn cài (vd `1.0.0` hay `1.0.0-rc.1`).

## Thứ được ký

| Thành phần của bản phát hành                                  | Chứng thực nguồn gốc build (SLSA) | Chữ ký cosign                  |
| ------------------------------------------------------------- | --------------------------------- | ------------------------------ |
| Image Docker `ghcr.io/thuongtamduy/autoxide:X.Y.Z`            | có, kèm chứng thực SBOM           | có, trên image                 |
| Binary Linux `autoxide-X.Y.Z-<target>.tar.gz`                 | có, cho từng file                 | qua `SHA256SUMS`               |
| SDK TypeScript `autoxide-sdk-X.Y.Z.tgz`                       | có                                | qua `SHA256SUMS`               |
| SBOM của image `sbom.spdx.json`                               | có                                | qua `SHA256SUMS`               |
| SBOM của các crate Rust `autoxide.cdx.json` (CycloneDX)       | có                                | qua `SHA256SUMS`               |
| `SHA256SUMS` (mã băm của các file trên)                       | —                                 | `SHA256SUMS.sigstore.json` (bundle Sigstore) |

Binary Linux, `SHA256SUMS` và chữ ký cosign có từ các bản sau 0.18.0. Bản cũ hơn chỉ có chứng thực của image.

Bạn cần [GitHub CLI](https://cli.github.com/) (`gh` 2.49 trở lên) hoặc
[cosign](https://docs.sigstore.dev/cosign/system_config/installation/) (3.0 trở lên). Một trong hai là đủ.

## Image Docker

Bằng GitHub CLI:

```sh
gh attestation verify oci://ghcr.io/thuongtamduy/autoxide:X.Y.Z --repo thuongtamduy/autoxide
```

Bằng cosign:

```sh
cosign verify ghcr.io/thuongtamduy/autoxide:X.Y.Z \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  --certificate-identity-regexp '^https://github\.com/thuongtamduy/autoxide/\.github/workflows/release(-image)?\.yml@refs/(tags/v.+|heads/main)$'
```

Danh tính cho phép hai workflow: `release.yml` (phát hành), và `release-image.yml` (người bảo trì chạy từ `main` để
đăng lại image của một tag phát hành đã có).

Thẻ như `X.Y.Z` về nguyên tắc có thể bị dời; digest thì không. Kiểm xong thì triển khai theo digest:

```sh
docker buildx imagetools inspect ghcr.io/thuongtamduy/autoxide:X.Y.Z --format '{{json .Manifest}}' | jq -r .digest
# rồi dùng ghcr.io/thuongtamduy/autoxide@sha256:… trong compose.yml hay manifest của bạn
```

## File đính kèm

Tải các file bạn cần từ trang phát hành, cùng `SHA256SUMS` và `SHA256SUMS.sigstore.json`, vào một thư mục.

Bằng GitHub CLI, kiểm thẳng từng file:

```sh
gh attestation verify autoxide-X.Y.Z-x86_64-unknown-linux-gnu.tar.gz --repo thuongtamduy/autoxide
```

Bằng cosign, kiểm chữ ký của danh sách mã băm, rồi kiểm file theo danh sách đó:

```sh
cosign verify-blob SHA256SUMS \
  --bundle SHA256SUMS.sigstore.json \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  --certificate-identity-regexp '^https://github\.com/thuongtamduy/autoxide/\.github/workflows/release\.yml@refs/tags/v.+$'
sha256sum --check --ignore-missing SHA256SUMS
```

Cả hai lệnh phải thành công. `sha256sum` in `OK` cho từng file bạn đã tải.

## Binary Linux

`autoxide-X.Y.Z-x86_64-unknown-linux-gnu.tar.gz` (Intel và AMD 64-bit) và
`autoxide-X.Y.Z-aarch64-unknown-linux-gnu.tar.gz` (ARM 64-bit) chứa đúng chương trình `autoxide` trong image Docker,
chép ra từ image, kèm file giấy phép. Nó build trên Debian 13 nên cần hệ Linux có thư viện C (glibc) mới ít nhất bằng
của Debian 13. `./autoxide --version` báo `GLIBC_2.… not found` thì dùng image Docker.

## Bản thử (release candidate)

Phiên bản như `1.0.0-rc.1` là **bản thử**: bản thử nghiệm ra trước một bản major. Chúng được ký và kiểm y như bản
chính thức. GitHub đánh dấu chúng là pre-release, và image của chúng chỉ có thẻ `X.Y.Z-rc.N`: không bao giờ `latest`
hay `X.Y`, nên bạn chỉ nhận bản thử khi gọi đúng tên. Không chạy bản thử ở production.

## Khi kiểm thất bại

Không chạy file hay image đó. Tải lại từ trang phát hành; vẫn thất bại thì báo riêng theo
[`SECURITY.md`](../SECURITY.vi.md).

# Verifying a release

**English** · [Tiếng Việt](VERIFYING.vi.md)

Every Autoxide release is built and signed by the `Release` workflow of this repository on GitHub Actions. There is
**no long-lived signing key**: [Sigstore](https://www.sigstore.dev/) issues a short-lived certificate tied to the
identity of the workflow run (GitHub's OIDC token), and the signature is recorded in the public transparency log
Rekor. Checking a signature tells you that the file or image was built by this repository's release workflow from a
release tag, and has not changed since. It does not tell you the code is free of bugs.

Replace `X.Y.Z` below with the version you install (for example `1.0.0` or `1.0.0-rc.1`).

## What is signed

| Release item                                                  | Build provenance (SLSA) | cosign signature                         |
| ------------------------------------------------------------- | ----------------------- | ---------------------------------------- |
| Docker image `ghcr.io/thuongtamduy/autoxide:X.Y.Z`            | yes, plus an SBOM attestation | yes, on the image                  |
| Linux binaries `autoxide-X.Y.Z-<target>.tar.gz`               | yes, for each file      | through `SHA256SUMS`                     |
| TypeScript SDK `autoxide-sdk-X.Y.Z.tgz`                       | yes                     | through `SHA256SUMS`                     |
| SBOM of the image `sbom.spdx.json`                            | yes                     | through `SHA256SUMS`                     |
| SBOM of the Rust crates `autoxide.cdx.json` (CycloneDX)         | yes                     | through `SHA256SUMS`                     |
| License texts of third-party crates `THIRD-PARTY-LICENSES.html` | yes                     | through `SHA256SUMS`                     |
| `SHA256SUMS` (checksums of the files above)                   | —                       | `SHA256SUMS.sigstore.json` (Sigstore bundle) |

Linux binaries, `SHA256SUMS` and the cosign signatures are attached to releases after 0.18.0. Older releases have
the image attestations only.

You need either the [GitHub CLI](https://cli.github.com/) (`gh` 2.49 or newer) or
[cosign](https://docs.sigstore.dev/cosign/system_config/installation/) (3.0 or newer). Either one is enough.

## The Docker image

With the GitHub CLI:

```sh
gh attestation verify oci://ghcr.io/thuongtamduy/autoxide:X.Y.Z --repo thuongtamduy/autoxide
```

With cosign:

```sh
cosign verify ghcr.io/thuongtamduy/autoxide:X.Y.Z \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  --certificate-identity-regexp '^https://github\.com/thuongtamduy/autoxide/\.github/workflows/release(-image)?\.yml@refs/(tags/v.+|heads/main)$'
```

The identity allows two workflows: `release.yml`, which publishes a release, and `release-image.yml`, which the
maintainers run from `main` to publish the image of an existing release tag again.

A tag such as `X.Y.Z` can in principle be moved; a digest cannot. After the check, deploy by digest:

```sh
docker buildx imagetools inspect ghcr.io/thuongtamduy/autoxide:X.Y.Z --format '{{json .Manifest}}' | jq -r .digest
# then use ghcr.io/thuongtamduy/autoxide@sha256:… in compose.yml or your manifests
```

## Release files

Download the files you want from the release page, together with `SHA256SUMS` and `SHA256SUMS.sigstore.json`, into
one folder.

With the GitHub CLI, check each file directly:

```sh
gh attestation verify autoxide-X.Y.Z-x86_64-unknown-linux-gnu.tar.gz --repo thuongtamduy/autoxide
```

With cosign, check the signature of the checksum list, then the files against it:

```sh
cosign verify-blob SHA256SUMS \
  --bundle SHA256SUMS.sigstore.json \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  --certificate-identity-regexp '^https://github\.com/thuongtamduy/autoxide/\.github/workflows/release\.yml@refs/tags/v.+$'
sha256sum --check --ignore-missing SHA256SUMS
```

Both commands must succeed. `sha256sum` prints `OK` for each file you downloaded.

### What a successful `gh attestation verify` looks like

In an interactive terminal it prints `✓ Verification succeeded!` followed by the attestation it matched. When its
output is not a terminal (a script, CI, a pipe, some editor terminals) it can print **nothing at all**: the answer is the
exit code. Success exits with `0`; any failure prints an error on stderr and exits with a non-zero code. To see the
result either way:

```sh
gh attestation verify autoxide-X.Y.Z-x86_64-unknown-linux-gnu.tar.gz --repo thuongtamduy/autoxide \
  && echo "VERIFIED" || echo "FAILED: do not run this file"
# or print the verified attestation as JSON, which is empty on failure:
gh attestation verify autoxide-X.Y.Z-x86_64-unknown-linux-gnu.tar.gz --repo thuongtamduy/autoxide --format json
```

## The Linux binaries

`autoxide-X.Y.Z-x86_64-unknown-linux-gnu.tar.gz` (64-bit Intel and AMD) and
`autoxide-X.Y.Z-aarch64-unknown-linux-gnu.tar.gz` (64-bit ARM) hold the same `autoxide` program as the Docker image,
copied out of it, plus the licence files. It is built on Debian 13, so it needs a Linux system with a C library
(glibc) at least as new as Debian 13's. If `./autoxide --version` fails with `GLIBC_2.… not found`, use the Docker
image instead.

## Release candidates

Versions such as `1.0.0-rc.1` are **release candidates**: test versions published before a major release. They are
signed and checked exactly like final releases. GitHub marks them as pre-releases, and their image has only the tag
`X.Y.Z-rc.N`: never `latest` and never `X.Y`, so you only get one if you ask for it by name. Do not run them in
production.

## If a check fails

Do not run the file or image. Download it again from the release page; if the check still fails, report it
privately as described in [`SECURITY.md`](../SECURITY.md).

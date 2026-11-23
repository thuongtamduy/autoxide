- Releases are signed without long-lived keys (Sigstore, through GitHub Actions OIDC): the Docker image has a cosign
  signature besides its build provenance and SBOM attestations, and every release file has a build provenance
  attestation plus a cosign-signed `SHA256SUMS`. How to check them: `docs/VERIFYING.md`.
- `SECURITY.md` describes the coordinated disclosure policy (GitHub Security Advisory, CVE, 90 days).

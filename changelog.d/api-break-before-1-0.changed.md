- Compatibility policy (`docs/COMPATIBILITY.md`): before 1.0 a minor release may break the HTTP API too,
  like every other contract; each break is listed under **Changed** or **Removed** with what to do
  instead. From 1.0.0 the policy applies in full and nothing in `/api/v1` breaks. The earlier wording
  said both that a `0.x` minor may break any contract and that the HTTP API already could not.

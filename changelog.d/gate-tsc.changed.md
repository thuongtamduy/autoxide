- Contributors: `./gate.sh` (including `GATE_LOCAL=off ./gate.sh --quick`) type-checks the TypeScript SDK and the guide
  examples with `tsc --noEmit` when Node.js is installed and the SDK, the contract or `docs/guides` changed; it clears
  the old extracted examples first, reinstalls `node_modules` when `package-lock.json` changed, and runs when the
  branch point cannot be found (shallow clone) (D-20261127-gate-tsc).

- The permission gate now requires a signed-in caller for every operation marked `x-permissions: []` (any signed-in
  caller), before the handler runs, even when the handler itself does not read the caller. Before, such an operation was
  protected only if its handler took the caller as input.

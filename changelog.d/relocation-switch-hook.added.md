- Kernel: modules can run a synchronous step when a tenant split or merge switches over
  (`Relocation::on_switch`, trait `tenancy::SwitchHook`). It runs in the same transaction that opens the new
  tenant; if it fails, the new tenant stays closed and the relocation can be run again.

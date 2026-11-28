- Rust API for modules: `StartContext::contribute` rejects a concrete type at start (a call without
  `::<dyn Trait>` was silently lost); a module that adds a relocation switch hook both with `Relocation::on_switch`
  and with `contribute::<dyn SwitchHook>` fails at start. Contributed switch hooks run after `on_switch` hooks.

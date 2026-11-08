//! A game-specific `axiom` executable: the full CLI (play, editor, acl, scenario, call, ...)
//! plus a native plugin, without modifying any engine crate.
//!
//! cargo run -p axiom --example custom_plugin -- --db data/spin.sqlite --pack examples/spin-pack play --start
mod plugin;

fn main() {
    axiom::main_with(axiom_runtime::installed_registry().with(plugin::SpinPlugin));
}

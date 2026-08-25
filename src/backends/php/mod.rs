//! PHP (ext-php-rs) binding generator backend for alef.

mod gen_bindings;
pub mod layout;
pub mod naming;
mod template_env;
pub mod trait_bridge;
mod type_map;

pub use gen_bindings::PhpBackend;
pub use gen_bindings::types::{is_php_prop_scalar, php_field_can_be_constructor_param};

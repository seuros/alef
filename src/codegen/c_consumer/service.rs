//! C symbols the FFI backend exports for a `[[services]]` surface.
//!
//! Every name here is derived from the emitters in
//! `backends::ffi::gen_bindings::service_api`, which write the `#[unsafe(no_mangle)] extern "C"`
//! items cbindgen turns into the header. Consumers of the C ABI (Go's cgo wrapper, the C
//! reference docs) must ask this module rather than re-compose the same string, because a
//! consumer that composes it independently links against a symbol nobody exported. ~keep

use heck::ToSnakeCase;

/// The prefix component every *service* symbol carries.
///
/// Unlike the free-function and method helpers in the parent module, which use `prefix`
/// verbatim, the service emitters apply `prefix.to_lowercase()`. Keeping that difference in one
/// named place is the point: a prefix with an internal capital spells two different symbols
/// under the two conventions, and only this one matches the service emitters. ~keep
fn prefix_component(prefix: &str) -> String {
    prefix.to_lowercase()
}

/// The service-name component every service symbol embeds.
///
/// `to_snake_case` (heck), not the acronym-aware `pascal_to_snake` the opaque-method helper
/// uses — the service emitters call `service.name.to_snake_case()`. ~keep
fn service_component(service_name: &str) -> String {
    service_name.to_snake_case()
}

/// The service constructor: `{prefix_lower}_{service_snake}_new`.
pub fn service_new_symbol(prefix: &str, service_name: &str) -> String {
    format!("{}_{}_new", prefix_component(prefix), service_component(service_name))
}

/// The service destructor: `{prefix_lower}_{service_snake}_free`.
pub fn service_free_symbol(prefix: &str, service_name: &str) -> String {
    format!("{}_{}_free", prefix_component(prefix), service_component(service_name))
}

/// A handler-registration entry point: `{prefix_lower}_{service_snake}_register_{method_snake}`.
pub fn service_register_symbol(prefix: &str, service_name: &str, method_name: &str) -> String {
    format!(
        "{}_{}_register_{}",
        prefix_component(prefix),
        service_component(service_name),
        method_name.to_snake_case()
    )
}

/// A registration *variant* shortcut or a configurator method, both of which the FFI backend
/// exports as `{prefix_lower}_{service_snake}_{method_snake}` with no infix.
pub fn service_method_symbol(prefix: &str, service_name: &str, method_name: &str) -> String {
    format!(
        "{}_{}_{}",
        prefix_component(prefix),
        service_component(service_name),
        method_name.to_snake_case()
    )
}

/// A run/finalize entry point: `{prefix_lower}_{service_snake}_ep_{method_snake}`.
pub fn service_entrypoint_symbol(prefix: &str, service_name: &str, method_name: &str) -> String {
    format!(
        "{}_{}_ep_{}",
        prefix_component(prefix),
        service_component(service_name),
        method_name.to_snake_case()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_new_and_free_bracket_the_service_component() {
        assert_eq!(service_new_symbol("demo", "HttpRouter"), "demo_http_router_new");
        assert_eq!(service_free_symbol("demo", "HttpRouter"), "demo_http_router_free");
    }

    #[test]
    fn service_register_symbol_carries_the_register_infix() {
        assert_eq!(
            service_register_symbol("demo", "HttpRouter", "add_handler"),
            "demo_http_router_register_add_handler"
        );
    }

    /// A variant shortcut and a configurator share the no-infix shape, which is exactly what
    /// separates them from [`service_register_symbol`] and [`service_entrypoint_symbol`]. ~keep
    #[test]
    fn service_method_symbol_has_no_infix() {
        assert_eq!(
            service_method_symbol("demo", "HttpRouter", "get"),
            "demo_http_router_get"
        );
        assert_ne!(
            service_method_symbol("demo", "HttpRouter", "get"),
            service_register_symbol("demo", "HttpRouter", "get")
        );
    }

    #[test]
    fn service_entrypoint_symbol_carries_the_ep_infix() {
        assert_eq!(
            service_entrypoint_symbol("demo", "HttpRouter", "run"),
            "demo_http_router_ep_run"
        );
    }

    /// The service emitters lowercase the prefix; the free-function/method helpers do not. A
    /// single-word lowercase prefix cannot tell the two apart, so this row uses one with an
    /// internal capital. ~keep
    #[test]
    fn service_symbols_lowercase_the_prefix() {
        assert_eq!(service_new_symbol("SampleCore", "Router"), "samplecore_router_new");
        assert_eq!(
            service_entrypoint_symbol("SampleCore", "Router", "run"),
            "samplecore_router_ep_run"
        );
    }
}

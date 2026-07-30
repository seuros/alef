use minijinja::Environment;

mod core_templates;
mod enum_error_templates;
mod rust_crate_templates;
mod streaming_templates;
mod swift_templates;
mod wrapper_templates;

static TEMPLATE_GROUPS: &[&[(&str, &str)]] = &[
    core_templates::TEMPLATES,
    enum_error_templates::TEMPLATES,
    wrapper_templates::TEMPLATES,
    rust_crate_templates::TEMPLATES,
    swift_templates::TEMPLATES,
    streaming_templates::TEMPLATES,
];

pub(crate) fn make_env() -> Environment<'static> {
    let mut env = Environment::new();
    env.set_trim_blocks(true);
    env.set_lstrip_blocks(true);
    env.set_keep_trailing_newline(true);
    for templates in TEMPLATE_GROUPS {
        for (name, src) in *templates {
            env.add_template(name, src).expect("built-in template is valid");
        }
    }
    env
}

pub(crate) fn render(template_name: &str, ctx: minijinja::Value) -> String {
    make_env()
        .get_template(template_name)
        .unwrap_or_else(|_| panic!("template {template_name} not found"))
        .render(ctx)
        .unwrap_or_else(|e| panic!("template {template_name} failed to render: {e}"))
}

#[cfg(test)]
mod tests {
    use super::render;

    /// The raw-pointer wrapper template must be registered in `TEMPLATE_GROUPS`.
    /// It is rendered from `gen_rust_crate::service_app_wrappers` for services
    /// with wrapper constructors; a template added to the directory but omitted
    /// from a group renders fine in unit tests that never hit its call site, then
    /// panics at generation time (`template <name> not found`). Rendering it here
    /// fails fast if it is ever dropped from the registry.
    #[test]
    fn raw_ptr_wrapper_template_is_registered_and_renders() {
        let out = render(
            "rust_wrapper_raw_ptr_fn.rs.jinja",
            minijinja::context! { wrapper_type => "RouteBuilder", fn_snake => "route_builder_raw_ptr" },
        );
        assert!(out.contains("route_builder_raw_ptr"), "fn name must be rendered: {out}");
        assert!(out.contains("RouteBuilder"), "wrapper type must be rendered: {out}");
    }
}

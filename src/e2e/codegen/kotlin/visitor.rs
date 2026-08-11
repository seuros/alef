use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::TypeDef;
use crate::e2e::fixture::{CallbackAction, VisitorSpec};
use heck::ToLowerCamelCase;
use std::collections::BTreeSet;

pub(super) fn attach_visitor(
    setup: &mut Vec<String>,
    args: &str,
    visitor: &VisitorSpec,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
) -> Option<String> {
    let bridge = config
        .trait_bridges
        .iter()
        .find(|bridge| bridge.options_type.is_some() && bridge.resolved_options_field().is_some())?;
    let trait_def = type_defs.iter().find(|type_def| type_def.name == bridge.trait_name)?;
    setup.push(format!("val visitor = object : {} {{", bridge.trait_name));
    for (name, action) in &visitor.callbacks {
        let method = trait_def.methods.iter().find(|method| method.name == *name)?;
        let parameters = method
            .params
            .iter()
            .map(|parameter| {
                let mut imports = BTreeSet::new();
                let ty = crate::backends::kotlin::kotlin_type_str_pub(&parameter.ty, parameter.optional, &mut imports);
                format!("{}: {ty}", parameter.name.to_lower_camel_case())
            })
            .collect::<Vec<_>>()
            .join(", ");
        setup.push(format!(
            "    override fun {}({parameters}): {} = {}",
            name.to_lower_camel_case(),
            bridge.result_type.as_deref().unwrap_or("VisitResult"),
            action_expression(action, method)
        ));
    }
    setup.push("}".to_string());
    let options_type = bridge.options_type.as_deref()?;
    let field = bridge.resolved_options_field()?.to_lower_camel_case();
    setup.push(format!("val options = {options_type}().copy({field} = visitor)"));
    Some(replace_options(args, options_type))
}

fn replace_options(args: &str, options_type: &str) -> String {
    for suffix in ["null".to_string(), format!("{options_type}()")] {
        if args == suffix {
            return "options".to_string();
        }
        if let Some(prefix) = args.strip_suffix(&format!(", {suffix}")) {
            return format!("{prefix}, options");
        }
    }
    if args.is_empty() {
        "options".to_string()
    } else {
        format!("{args}, options")
    }
}

fn action_expression(action: &CallbackAction, method: &crate::core::ir::MethodDef) -> String {
    match action {
        CallbackAction::Skip => "VisitResult.skip()".to_string(),
        CallbackAction::Continue => "VisitResult.continue_()".to_string(),
        CallbackAction::PreserveHtml => "VisitResult.preserveHtml()".to_string(),
        CallbackAction::Custom { output } => {
            format!("VisitResult.custom(\"{}\")", crate::e2e::escape::escape_kotlin(output))
        }
        CallbackAction::CustomTemplate { template, .. } => {
            let mut expression = crate::e2e::escape::escape_kotlin(template);
            for parameter in &method.params {
                let name = parameter.name.to_lower_camel_case();
                expression = expression.replace(&format!("{{{}}}", parameter.name), &format!("${name}"));
            }
            format!("VisitResult.custom(\"{expression}\")")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{attach_visitor, replace_options};
    use crate::core::config::{ResolvedCrateConfig, TraitBridgeConfig};
    use crate::core::ir::{MethodDef, ParamDef, TypeDef, TypeRef};
    use crate::e2e::fixture::{CallbackAction, VisitorSpec};
    use std::collections::BTreeMap;

    #[test]
    fn replaces_default_options_argument() {
        assert_eq!(replace_options("html, null", "Options"), "html, options");
        assert_eq!(replace_options("html, Options()", "Options"), "html, options");
    }

    #[test]
    fn emits_trait_override_and_attaches_options() {
        let visitor = VisitorSpec {
            callbacks: BTreeMap::from([("visit_text".into(), CallbackAction::Skip)]),
        };
        let config = ResolvedCrateConfig {
            trait_bridges: vec![TraitBridgeConfig {
                trait_name: "TextVisitor".into(),
                options_type: Some("RenderOptions".into()),
                options_field: Some("visitor".into()),
                result_type: Some("VisitResult".into()),
                ..TraitBridgeConfig::default()
            }],
            ..ResolvedCrateConfig::default()
        };
        let types = [TypeDef {
            name: "TextVisitor".into(),
            is_trait: true,
            methods: vec![MethodDef {
                name: "visit_text".into(),
                params: vec![ParamDef {
                    name: "text".into(),
                    ty: TypeRef::String,
                    ..ParamDef::default()
                }],
                return_type: TypeRef::Named("VisitResult".into()),
                ..MethodDef::default()
            }],
            ..TypeDef::default()
        }];
        let mut setup = Vec::new();

        let args = attach_visitor(&mut setup, "html, null", &visitor, &config, &types).expect("visitor metadata");
        let rendered = setup.join("\n");
        assert_eq!(args, "html, options");
        assert!(rendered.contains("object : TextVisitor"), "{rendered}");
        assert!(
            rendered.contains("override fun visitText(text: String): VisitResult"),
            "{rendered}"
        );
        assert!(
            rendered.contains("RenderOptions().copy(visitor = visitor)"),
            "{rendered}"
        );
        if std::process::Command::new("kotlinc").arg("-version").output().is_ok() {
            let directory = tempfile::tempdir().expect("temporary Kotlin project");
            let source = format!(
                "interface VisitResult {{ companion object {{ fun skip(): VisitResult = object : VisitResult {{}} }} }}\n\
                 interface TextVisitor {{ fun visitText(text: String): VisitResult = VisitResult.skip() }}\n\
                 data class RenderOptions(val visitor: TextVisitor? = null)\n\
                 fun render(html: String, options: RenderOptions) = html + options.toString()\n\
                 fun main() {{\n{}\nrender(\"sample\", options)\n}}",
                setup
                    .iter()
                    .map(|line| format!("    {line}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            let path = directory.path().join("Snippet.kt");
            std::fs::write(&path, source).expect("write Kotlin snippet");
            let output = std::process::Command::new("kotlinc")
                .arg(&path)
                .arg("-d")
                .arg(directory.path().join("snippet.jar"))
                .output()
                .expect("run kotlinc");
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        }
    }
}

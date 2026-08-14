use crate::core::version::to_dotnet_assembly_version;

pub(super) fn sync_csharp_project_versions(content: &str, version: &str) -> Option<String> {
    let assembly_version = to_dotnet_assembly_version(version);
    let replacements = [
        ("Version", version),
        ("InformationalVersion", version),
        ("AssemblyVersion", assembly_version.as_str()),
        ("FileVersion", assembly_version.as_str()),
    ];
    let mut working = content.to_string();
    for (element, replacement) in replacements {
        if let Some(rewritten) = replace_xml_element(&working, element, replacement) {
            working = rewritten;
        }
    }
    (working != content).then_some(working)
}

fn replace_xml_element(content: &str, element: &str, value: &str) -> Option<String> {
    let pattern = format!(r"<{element}>[^<]*</{element}>");
    let regex = regex::Regex::new(&pattern).ok()?;
    let replacement = format!("<{element}>{value}</{element}>");
    let rewritten = regex.replace_all(content, replacement).into_owned();
    (rewritten != content).then_some(rewritten)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syncs_semver_and_numeric_csharp_versions() {
        let input = concat!(
            "<Version>1.2.2</Version>\n",
            "<AssemblyVersion>1.2.2.0</AssemblyVersion>\n",
            "<FileVersion>1.2.2.0</FileVersion>\n",
            "<InformationalVersion>1.2.2</InformationalVersion>\n",
        );
        let output = sync_csharp_project_versions(input, "1.2.3-rc.4").expect("versions changed");
        assert!(output.contains("<Version>1.2.3-rc.4</Version>"));
        assert!(output.contains("<InformationalVersion>1.2.3-rc.4</InformationalVersion>"));
        assert!(output.contains("<AssemblyVersion>1.2.3.0</AssemblyVersion>"));
        assert!(output.contains("<FileVersion>1.2.3.0</FileVersion>"));
    }

    #[test]
    fn already_current_csharp_versions_are_unchanged() {
        let input = concat!(
            "<Version>1.2.3-rc.4</Version>\n",
            "<AssemblyVersion>1.2.3.0</AssemblyVersion>\n",
            "<FileVersion>1.2.3.0</FileVersion>\n",
            "<InformationalVersion>1.2.3-rc.4</InformationalVersion>\n",
        );
        assert_eq!(sync_csharp_project_versions(input, "1.2.3-rc.4"), None);
    }
}

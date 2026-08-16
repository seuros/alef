use minijinja::Environment;

mod files;
mod inline;

const TEMPLATE_GROUPS: &[&[(&str, &str)]] = &[inline::TEMPLATES, files::TEMPLATES];

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
mod template_registration_tests {
    use super::TEMPLATE_GROUPS;
    use std::collections::HashSet;
    use std::path::Path;

    /// `render()` resolves names against `TEMPLATE_GROUPS`, not the filesystem, so a
    /// `.jinja` file added to `templates/` but never wired into `files::TEMPLATES` (or given
    /// an inline entry in `inline::TEMPLATES`) compiles fine (`include_str!` only runs for
    /// entries that are listed) and panics only once an emitter reaches it at generation
    /// time. Compare by content rather than by registered key: some entries in
    /// `inline::TEMPLATES` are inline literal strings with no backing file at all, and a
    /// registered key may be aliased away from the filename, but every file's bytes must
    /// appear in `TEMPLATE_GROUPS` somewhere. ~keep
    #[test]
    fn every_template_file_is_registered() {
        let templates_dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src/backends/rustler/templates"));
        let registered_contents: HashSet<&str> = TEMPLATE_GROUPS
            .iter()
            .flat_map(|group| group.iter())
            .map(|(_, content)| *content)
            .collect();

        let mut unregistered = Vec::new();
        collect_unregistered(templates_dir, templates_dir, &registered_contents, &mut unregistered);
        unregistered.sort();
        assert!(
            unregistered.is_empty(),
            "found .jinja file(s) in templates/ whose content is not registered in TEMPLATE_GROUPS: {unregistered:?}"
        );
    }

    fn collect_unregistered(
        root: &Path,
        dir: &Path,
        registered_contents: &HashSet<&str>,
        unregistered: &mut Vec<String>,
    ) {
        for entry in std::fs::read_dir(dir).expect("read templates directory") {
            let entry = entry.expect("read templates directory entry");
            let path = entry.path();
            if path.is_dir() {
                collect_unregistered(root, &path, registered_contents, unregistered);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("jinja") {
                continue;
            }
            let content = std::fs::read_to_string(&path).expect("read template file");
            if !registered_contents.contains(content.as_str()) {
                let relative = path
                    .strip_prefix(root)
                    .expect("template path under templates root")
                    .to_str()
                    .expect("template path is valid UTF-8")
                    .replace('\\', "/");
                unregistered.push(relative);
            }
        }
    }
}

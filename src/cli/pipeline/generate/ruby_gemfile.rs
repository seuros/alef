use crate::core::backend::GeneratedFile;
use anyhow::Context as _;
use std::path::Path;

/// Ordinary scaffold seeds are create-once, but an emitted Ruby Gemfile must refresh Alef's
/// managed gem constraints without deleting consumer additions. Identifying that exception before
/// the generic skip and ownership guards lets normal, non-clean generation converge it. ~keep
pub(super) fn is_merge_target(file: &GeneratedFile, full_path: &Path) -> bool {
    file.path.file_name().is_some_and(|name| name == "Gemfile")
        && file.content.contains("gem \"rb_sys\"")
        && full_path.exists()
}

pub(super) fn merge_file(file: &GeneratedFile, full_path: &Path) -> anyhow::Result<String> {
    let existing = std::fs::read_to_string(full_path)
        .with_context(|| format!("failed to read existing {}", full_path.display()))?;
    Ok(merge(&existing, &file.content))
}

pub(super) fn merge(existing: &str, generated: &str) -> String {
    let managed = generated.lines().filter_map(managed_gem);
    let mut lines: Vec<String> = existing.lines().map(str::to_string).collect();
    let mut missing = Vec::new();
    for (name, generated_line) in managed {
        let double = format!("gem \"{name}\"");
        let single = format!("gem '{name}'");
        if let Some(line) = lines
            .iter_mut()
            .find(|line| line.trim_start().starts_with(&double) || line.trim_start().starts_with(&single))
        {
            let indent = &line[..line.len() - line.trim_start().len()];
            *line = format!("{indent}{generated_line}");
        } else {
            missing.push(format!("  {generated_line}"));
        }
    }
    insert_missing_gems(&mut lines, missing);
    format!("{}\n", lines.join("\n"))
}

fn managed_gem(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim();
    let name = trimmed.strip_prefix("gem \"")?.split_once('"')?.0;
    Some((name, trimmed))
}

fn insert_missing_gems(lines: &mut Vec<String>, missing: Vec<String>) {
    if missing.is_empty() {
        return;
    }
    let insert_at = lines
        .iter()
        .position(|line| line.trim() == "group :development do")
        .map(|start| start + 1);
    if let Some(index) = insert_at {
        lines.splice(index..index, missing);
        return;
    }
    lines.push(String::new());
    lines.push("group :development do".to_string());
    lines.extend(missing);
    lines.push("end".to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refreshes_managed_constraints_and_preserves_extras() {
        let existing = r#"source "https://rubygems.org"

gemspec

group :development do
  gem "rb_sys", ">= 0.9", "< 0.9.128"
  gem "debug", "~> 1.9"
end
"#;
        let generated = r#"source "https://rubygems.org"

gemspec

group :development do
  gem "rake-compiler", "~> 1.3"
  gem "rb_sys", ">= 0.9.130"
end
"#;

        let merged = merge(existing, generated);

        assert!(merged.contains("gem \"rb_sys\", \">= 0.9.130\""), "{merged}");
        assert!(!merged.contains("< 0.9.128"), "{merged}");
        assert!(merged.contains("gem \"debug\", \"~> 1.9\""), "{merged}");
        assert!(merged.contains("gem \"rake-compiler\", \"~> 1.3\""), "{merged}");
    }

    #[test]
    fn inserts_missing_gem_after_nested_development_block() {
        let existing = "group :development do\n  platforms :mri do\n    gem \"debug\"\n  end\nend\n";
        let generated = "group :development do\n  gem \"rb_sys\", \">= 0.9.130\"\nend\n";

        let merged = merge(existing, generated);

        assert!(
            merged.starts_with("group :development do\n  gem \"rb_sys\", \">= 0.9.130\"\n  platforms :mri do"),
            "managed gems must be inserted before nested blocks in the development group: {merged}"
        );
    }

    #[test]
    fn normal_scaffold_run_refreshes_emitted_gemfile() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("packages/ruby/Gemfile");
        std::fs::create_dir_all(path.parent().expect("Gemfile parent")).expect("create parent");
        std::fs::write(
            &path,
            "source \"https://rubygems.org\"\n\ngemspec\n\ngroup :development do\n  gem \"rb_sys\", \">= 0.9\", \"< 0.9.128\"\n  gem \"debug\", \"~> 1.9\"\nend\n",
        )
        .expect("write stale Gemfile");
        let file = GeneratedFile {
            path: std::path::PathBuf::from("packages/ruby/Gemfile"),
            content: "source \"https://rubygems.org\"\n\ngemspec\n\ngroup :development do\n  gem \"rake-compiler\", \"~> 1.3\"\n  gem \"rb_sys\", \">= 0.9.130\"\nend\n".to_string(),
            generated_header: false,
        };

        let report =
            super::super::write_scaffold_files_report(&[file], temp.path(), false).expect("normal scaffold write");
        let refreshed = std::fs::read_to_string(path).expect("read refreshed Gemfile");

        assert_eq!(
            report.changed_count(),
            1,
            "normal generation must refresh the emitted Gemfile"
        );
        assert!(refreshed.contains("gem \"rb_sys\", \">= 0.9.130\""), "{refreshed}");
        assert!(refreshed.contains("gem \"debug\", \"~> 1.9\""), "{refreshed}");
    }
}

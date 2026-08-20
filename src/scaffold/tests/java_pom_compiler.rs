use super::*;

/// Extracts the `<plugin>` block for `artifact_id` out of a rendered pom.xml.
fn plugin_section<'a>(pom_content: &'a str, artifact_id: &str) -> &'a str {
    pom_content
        .split(&format!("<artifactId>{artifact_id}</artifactId>"))
        .nth(1)
        .and_then(|section| section.split("</plugin>").next())
        .unwrap_or_else(|| panic!("pom.xml must configure {artifact_id}"))
}

/// Extracts the first `<excludes>...</excludes>` block (inclusive) out of `section`.
fn excludes_block<'a>(section: &'a str) -> &'a str {
    let start = section
        .find("<excludes>")
        .unwrap_or_else(|| panic!("expected an <excludes> block in:\n{section}"));
    let end = section[start..]
        .find("</excludes>")
        .unwrap_or_else(|| panic!("<excludes> block is not well-formed in:\n{section}"))
        + start
        + "</excludes>".len();
    &section[start..end]
}

/// Regression: `<sourceDirectory>${project.basedir}</sourceDirectory>` (the flat layout alef
/// emits) makes the compiler walk the WHOLE project basedir, not just alef-emitted sources.
/// Without excludes, `.alef/snippets/sessions/<hash>/Example.java` scratch files written by
/// doc-snippet validation collide as duplicate top-level classes across sessions and break
/// `mvn compile`; `src/test/java/**` and `target/**` are swept in for the same reason
/// maven-source-plugin and maven-javadoc-plugin restrict themselves elsewhere in this pom. ~keep
#[test]
fn test_scaffold_java_compiler_plugin_excludes_test_scratch_and_target_dirs() {
    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Java]).unwrap();
    let files = language_files(&all_files);
    let pom = files.iter().find(|f| f.path.ends_with("pom.xml")).unwrap();

    let compiler_section = plugin_section(&pom.content, "maven-compiler-plugin");
    let excludes = excludes_block(compiler_section);

    assert!(
        excludes.contains("<exclude>src/test/java/**</exclude>"),
        "compiler plugin must exclude hand-written test sources; block:\n{excludes}"
    );
    assert!(
        excludes.contains("<exclude>.alef/**</exclude>"),
        "compiler plugin must exclude the .alef/ snippet-validation scratch directory; block:\n{excludes}"
    );
    assert!(
        excludes.contains("<exclude>target/**</exclude>"),
        "compiler plugin must exclude its own build output directory; block:\n{excludes}"
    );
}

/// Bite test: builds the scaffolded pom in a real temp Maven project with two `.alef` snippet
/// sessions that each emit a same-named `Example.java` (the actual shape alef's doc-snippet
/// validation scratch takes). Compiles once with the pom as generated (must succeed) and once
/// with the `<excludes>` block programmatically stripped out (must fail with the real
/// `duplicate class` error), proving the fix is load-bearing rather than vacuous. Skips when
/// `mvn` is unavailable.
#[test]
fn test_scaffold_java_compiler_plugin_excludes_prevent_alef_scratch_duplicate_class_collision() {
    if std::process::Command::new("mvn").arg("--version").output().is_err() {
        return;
    }

    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Java]).unwrap();
    let files = language_files(&all_files);
    let pom = files.iter().find(|f| f.path.ends_with("pom.xml")).unwrap();

    let group_id = config.java_group_id();
    let source_root = group_id.split('.').next().unwrap_or("dev");

    let compile = |pom_content: &str| -> std::process::Output {
        let project_dir = tempfile::tempdir().expect("temp project dir");
        std::fs::write(project_dir.path().join("pom.xml"), pom_content).expect("write pom");

        let package_dir = project_dir.path().join(source_root).join("scratchcompile");
        std::fs::create_dir_all(&package_dir).expect("package dir");
        std::fs::write(
            package_dir.join("RealSource.java"),
            "package scratchcompile;\n\npublic final class RealSource {\n}\n",
        )
        .expect("write real source");

        for hash in ["deadbeefcafefeed", "feedfacecafebabe"] {
            let scratch_dir = project_dir.path().join(".alef/snippets/sessions").join(hash);
            std::fs::create_dir_all(&scratch_dir).expect("scratch session dir");
            std::fs::write(scratch_dir.join("Example.java"), "public final class Example {\n}\n")
                .expect("write scratch source");
        }

        std::process::Command::new("mvn")
            // Not `-o`: see the comment on the checkstyle bite test above -- same reason.
            .args(["-q", "compile", "-Dcheckstyle.skip=true"])
            .current_dir(project_dir.path())
            .output()
            .expect("mvn runs")
    };

    let fixed_output = compile(&pom.content);
    assert!(
        fixed_output.status.success(),
        "compiler-plugin excludes must keep colliding .alef scratch sources out of `mvn compile`; \
         stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&fixed_output.stdout),
        String::from_utf8_lossy(&fixed_output.stderr),
    );

    let compiler_section = plugin_section(&pom.content, "maven-compiler-plugin");
    let excludes = excludes_block(compiler_section);
    let broken_pom = pom.content.replacen(excludes, "", 1);

    let broken_output = compile(&broken_pom);
    let broken_diagnostics = format!(
        "{}{}",
        String::from_utf8_lossy(&broken_output.stdout),
        String::from_utf8_lossy(&broken_output.stderr)
    );
    assert!(
        !broken_output.status.success(),
        "without the compiler-plugin excludes, colliding .alef scratch classes should fail \
         `mvn compile`; stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&broken_output.stdout),
        String::from_utf8_lossy(&broken_output.stderr),
    );
    assert!(
        broken_diagnostics.contains("duplicate class"),
        "the failure without excludes must be the duplicate-class collision this fix targets, \
         not some other break; diagnostics:\n{broken_diagnostics}"
    );
}

/// Regression, verified independently against a real `mvn javadoc:javadoc` run rather than
/// trusting the existing string-assertion coverage in `ffi_go_java_ruby.rs`: without
/// `<sourceFileIncludes>`, `<sourcepath>${project.basedir}</sourcepath>` sweeps
/// `src/test/java/**` into the javadoc scan. Test sources import JUnit, which is test-scoped
/// and absent from the javadoc classpath, so `attach-javadocs` fails with
/// `package org.junit.jupiter.api does not exist` for any consumer with Java tests. Compiles
/// once with the pom as generated (must succeed) and once with `<sourceFileIncludes>`
/// programmatically stripped out (must reproduce that exact failure). Skips when `mvn` is
/// unavailable.
#[test]
fn test_scaffold_java_javadoc_plugin_source_file_includes_prevent_test_source_leak() {
    if std::process::Command::new("mvn").arg("--version").output().is_err() {
        return;
    }

    let config = test_config();
    let api = test_api();
    let all_files = scaffold(&api, &config, &[Language::Java]).unwrap();
    let files = language_files(&all_files);
    let pom = files.iter().find(|f| f.path.ends_with("pom.xml")).unwrap();

    let group_id = config.java_group_id();
    let source_root = group_id.split('.').next().unwrap_or("dev");

    let run_javadoc = |pom_content: &str| -> std::process::Output {
        let project_dir = tempfile::tempdir().expect("temp project dir");
        std::fs::write(project_dir.path().join("pom.xml"), pom_content).expect("write pom");

        let package_dir = project_dir.path().join(source_root).join("scratchjavadoc");
        std::fs::create_dir_all(&package_dir).expect("main package dir");
        std::fs::write(
            package_dir.join("RealSource.java"),
            "package scratchjavadoc;\n\npublic final class RealSource {\n    public int value() {\n        return 1;\n    }\n}\n",
        )
        .expect("write real source");

        let test_package_dir = project_dir.path().join("src/test/java/scratchjavadoc");
        std::fs::create_dir_all(&test_package_dir).expect("test package dir");
        std::fs::write(
            test_package_dir.join("RealSourceTest.java"),
            "package scratchjavadoc;\n\nimport org.junit.jupiter.api.Test;\n\nclass RealSourceTest {\n    @Test\n    void works() {\n    }\n}\n",
        )
        .expect("write test source");

        std::process::Command::new("mvn")
            // Not `-o`: see the comment on the checkstyle bite test above -- same reason.
            // -Dcheckstyle.skip=true: a direct goal invocation still runs earlier
            // phase-bound executions (checkstyle is bound to `validate`), and this test
            // is isolating the javadoc plugin's own behavior, not checkstyle's.
            .args(["-q", "javadoc:javadoc", "-Dcheckstyle.skip=true"])
            .current_dir(project_dir.path())
            .output()
            .expect("mvn runs")
    };

    let fixed_output = run_javadoc(&pom.content);
    assert!(
        fixed_output.status.success(),
        "javadoc plugin must build cleanly against a consumer with Java tests; stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&fixed_output.stdout),
        String::from_utf8_lossy(&fixed_output.stderr),
    );

    let javadoc_section = plugin_section(&pom.content, "maven-javadoc-plugin");
    let start = javadoc_section
        .find("<sourceFileIncludes>")
        .expect("javadoc plugin must declare <sourceFileIncludes>");
    let end = javadoc_section[start..]
        .find("</sourceFileIncludes>")
        .expect("<sourceFileIncludes> block is not well-formed")
        + start
        + "</sourceFileIncludes>".len();
    let source_file_includes = &javadoc_section[start..end];
    let broken_pom = pom.content.replacen(source_file_includes, "", 1);

    let broken_output = run_javadoc(&broken_pom);
    let broken_diagnostics = format!(
        "{}{}",
        String::from_utf8_lossy(&broken_output.stdout),
        String::from_utf8_lossy(&broken_output.stderr)
    );
    assert!(
        !broken_output.status.success(),
        "without sourceFileIncludes, javadoc should sweep in src/test/java and fail; \
         stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&broken_output.stdout),
        String::from_utf8_lossy(&broken_output.stderr),
    );
    assert!(
        broken_diagnostics.contains("org.junit.jupiter.api does not exist"),
        "the failure without sourceFileIncludes must be the test-source classpath leak this \
         fix targets, not some other break; diagnostics:\n{broken_diagnostics}"
    );
}

use crate::core::backend::GeneratedFile;
use crate::core::config::{KotlinTarget, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use crate::core::template_versions::{maven, toolchain};
use crate::scaffold::{parse_author, scaffold_meta};
use anyhow::Context as _;

use std::path::{Path, PathBuf};

pub(crate) fn scaffold_kotlin(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    if let Some(mode) = config.kotlin.as_ref().and_then(|k| k.mode.as_deref()) {
        return match mode {
            "android" => anyhow::bail!(
                "`[crates.kotlin] mode = \"android\"` was removed in alef 0.16. \
                 Use `Language::KotlinAndroid` (slug `\"kotlin_android\"`) and the \
                 `alef-backend-kotlin-android` crate instead."
            ),
            "kmp" => scaffold_kotlin_multiplatform(api, config),
            _ => scaffold_kotlin_jvm(api, config),
        };
    }
    if config.kotlin.as_ref().is_some_and(|k| k.target == KotlinTarget::Native) {
        return scaffold_kotlin_native(api, config);
    }
    if config
        .kotlin
        .as_ref()
        .is_some_and(|k| k.target == KotlinTarget::Multiplatform)
    {
        return scaffold_kotlin_multiplatform(api, config);
    }

    scaffold_kotlin_jvm(api, config)
}

fn scaffold_kotlin_jvm(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let version = &api.version;
    let kotlin_package = config.kotlin_package();
    let kotlin_package_path = kotlin_package.replace('.', "/");
    let project_name = config.name.replace('-', "_");

    let kotlin_plugin = maven::KOTLIN_JVM_PLUGIN;
    let kotlinx_coroutines = maven::KOTLINX_COROUTINES_CORE;
    let jna = maven::JNA;
    let junit_legacy = maven::JUNIT_LEGACY;
    let jackson = maven::JACKSON;
    let jackson_annotations = maven::JACKSON_ANNOTATIONS;
    let jspecify = maven::JSPECIFY;
    let jvm_target = toolchain::KOTLIN_JVM_TARGET;
    let kotlin_artifact_id = format!("{}-kotlin", config.name);

    let vanniktech = maven::VANNIKTECH_MAVEN_PUBLISH;
    let repo_url = meta.configured_repository.clone().ok_or_else(|| {
        anyhow::anyhow!(
            "Kotlin scaffold requires package metadata repository; set package_metadata.repository or scaffold.repository"
        )
    })?;
    if meta.authors.is_empty() {
        anyhow::bail!(
            "Kotlin scaffold requires package metadata authors; set package_metadata.authors or scaffold.authors"
        );
    }
    let license = meta.license.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "Kotlin scaffold requires package metadata license; set package_metadata.license or scaffold.license"
        )
    })?;
    let scm = scm_urls(&repo_url);
    let license_url = match license {
        "Elastic-2.0" => "https://www.elastic.co/licensing/elastic-license",
        "MIT" => "https://opensource.org/licenses/MIT",
        "Apache-2.0" => "https://www.apache.org/licenses/LICENSE-2.0",
        _ => "",
    };
    let kt = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"").replace('$', "\\$");
    let licenses_block = if license_url.is_empty() {
        format!(
            "    licenses {{\n      license {{\n        name.set(\"{}\")\n      }}\n    }}\n",
            kt(license)
        )
    } else {
        format!(
            "    licenses {{\n      license {{\n        name.set(\"{}\")\n        url.set(\"{}\")\n      }}\n    }}\n",
            kt(license),
            kt(license_url)
        )
    };
    let developers_block = if meta.authors.is_empty() {
        String::new()
    } else {
        let devs: Vec<String> = meta
            .authors
            .iter()
            .map(|a| {
                let (name, email) = parse_author(a);
                format!(
                    "      developer {{\n        name.set(\"{}\")\n        email.set(\"{}\")\n      }}",
                    kt(name),
                    kt(email)
                )
            })
            .collect();
        format!("    developers {{\n{}\n    }}\n", devs.join("\n"))
    };
    let description = kt(&meta.description);
    let repo_url = kt(&repo_url);
    let scm_connection = kt(&scm.connection);
    let scm_developer_connection = kt(&scm.developer_connection);

    let build_gradle = format!(
        r#"import com.vanniktech.maven.publish.JavadocJar
import com.vanniktech.maven.publish.KotlinJvm
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

buildscript {{
  dependencies {{
    classpath("com.vanniktech:gradle-maven-publish-plugin:{vanniktech}")
  }}
}}

plugins {{
  `java-library`
  kotlin("jvm") version "{kotlin_plugin}"
  id("com.vanniktech.maven.publish") version "{vanniktech}"
}}

group = "{package}"
version = "{version}"

repositories {{
  mavenCentral()
}}

dependencies {{
  api("net.java.dev.jna:jna:{jna}")
  // Jackson is on the public surface because the alef-emitted Java records
  // include `@JsonProperty` annotations for serialization round-tripping.
  api("com.fasterxml.jackson.core:jackson-annotations:{jackson_annotations}")
  api("com.fasterxml.jackson.core:jackson-databind:{jackson}")
  api("com.fasterxml.jackson.datatype:jackson-datatype-jdk8:{jackson}")
  // jspecify ships the `@Nullable` / `@NonNull` annotations referenced by the
  // alef-emitted Java facade; it must be on the api configuration so Kotlin
  // consumers see the annotations on cross-language types.
  api("org.jspecify:jspecify:{jspecify}")
  implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:{kotlinx_coroutines}")
  testImplementation("org.jetbrains.kotlin:kotlin-test:{kotlin_plugin}")
  testImplementation("junit:junit:{junit_legacy}")
}}

java {{
  sourceCompatibility = JavaVersion.VERSION_{jvm_target}
  targetCompatibility = JavaVersion.VERSION_{jvm_target}
}}

// Include the alef-emitted Java facade (sibling package) so the Kotlin object
// can call into the JNA-loaded native bridge. The Kotlin backend places its
// generated files in a sub-package (`<group>.kt`) to avoid colliding with the
// Java facade that uses the canonical `<group>` package.
sourceSets {{
  main {{
    java {{
      // Pull in the Java facade emitted by the alef Java backend so the
      // Kotlin module compiles against the same on-disk sources. The alef
      // Java backend writes to `packages/java/` (package-root layout), not
      // the Maven `src/main/java/` convention.
      srcDir("../java")
    }}
  }}
}}

kotlin {{
  compilerOptions {{
    jvmTarget.set(JvmTarget.JVM_{jvm_target})
  }}
}}

// JNA needs the native lib on java.library.path; default to the workspace
// `target/release` cargo output. Override with `-Pnative.lib.path=<dir>`.
tasks.withType<Test>().configureEach {{
  val libPath = (project.findProperty("native.lib.path") as String?) ?: "$rootDir/../../target/release"
  systemProperty("jna.library.path", libPath)
  systemProperty("java.library.path", libPath)
  useJUnit()
}}

// Publish to Maven Central via the vanniktech plugin: signs all publications
// and uploads with publishingType=AUTOMATIC, so `publishAndReleaseToMavenCentral`
// auto-releases the Central Portal deployment (the bare `maven-publish` plugin
// can only stage, leaving the artifact unreleased). The Kotlin-specific
// artifactId disambiguates this module from the sibling Java facade in the same
// Maven group; the version is inherited from the top-level `version` above
// (kept current by `alef sync-versions`), so it is omitted from `coordinates`.
mavenPublishing {{
  configure(
    KotlinJvm(
      javadocJar = JavadocJar.Empty(),
      sourcesJar = true,
    ),
  )

  publishToMavenCentral()
  signAllPublications()

  coordinates(
    groupId = "{package}",
    artifactId = "{kotlin_artifact_id}",
  )

  pom {{
    name.set("{kotlin_artifact_id}")
    description.set("{description}")
    url.set("{repo_url}")
{licenses_block}{developers_block}    scm {{
      url.set("{repo_url}")
      connection.set("{scm_connection}")
      developerConnection.set("{scm_developer_connection}")
    }}
  }}
}}
"#,
        package = kotlin_package,
        version = version,
        jackson = jackson,
        jspecify = jspecify,
        kotlin_artifact_id = kotlin_artifact_id,
        scm_connection = scm_connection,
        scm_developer_connection = scm_developer_connection,
    );

    let settings_gradle = format!("rootProject.name = \"{project_name}\"\n");

    let gitignore = "build/\n.gradle/\n.idea/\n*.iml\n";

    let editorconfig = "[*]\ncharset = utf-8\nend_of_line = lf\ninsert_final_newline = true\ntrim_trailing_whitespace = true\n\n\
[*.kt]\nindent_style = space\nindent_size = 4\n\n\
[*.gradle.kts]\nindent_style = space\nindent_size = 2\n";

    let gradle_properties = "org.gradle.parallel=true\nkotlin.code.style=official\n";

    let readme = format!(
        r#"# {project_name}

{description}

## Installation

Add to your `build.gradle.kts`:

```kotlin
dependencies {{
    implementation("{package}:{kotlin_artifact_id}:{version}")
}}
```

## Building

```sh
gradle build
gradle test
```

## License

{license}
"#,
        project_name = project_name,
        description = meta.description,
        package = kotlin_package,
        kotlin_artifact_id = kotlin_artifact_id,
        version = version,
        license = license,
    );

    let sample_kotlin = format!(
        r#"package {package}.sample

// Sample usage of the generated Kotlin bindings.
// Replace with your actual API calls after code generation.

object Sample {{
    @JvmStatic
    fun main(args: Array<String>) {{
        println("Sample: {project_name} bindings loaded successfully")
    }}
}}
"#,
        package = kotlin_package,
        project_name = project_name,
    );

    Ok(vec![
        GeneratedFile {
            path: PathBuf::from("packages/kotlin/build.gradle.kts"),
            content: build_gradle,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/kotlin/settings.gradle.kts"),
            content: settings_gradle,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/kotlin/.gitignore"),
            content: gitignore.to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/kotlin/.editorconfig"),
            content: editorconfig.to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/kotlin/gradle.properties"),
            content: gradle_properties.to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/kotlin/README.md"),
            content: readme,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from(format!(
                "packages/kotlin/src/main/kotlin/{kotlin_package_path}/sample/Sample.kt"
            )),
            content: sample_kotlin,
            generated_header: false,
        },
    ])
}

/// The stale `sourceSets { main { ... } }` sub-block [`scaffold_kotlin_jvm`] emitted before the
/// fix that dropped it (the alef Kotlin backend already writes binding sources under the
/// standard `src/main/kotlin/` layout, so this extra root `srcDir(".")` only dragged `build/`
/// into vanniktech's sources jar, tripping Gradle 9's output-overlap validation and breaking
/// `publishToMavenCentral`). A fixed, static block with no per-project variables — every
/// scaffolded Kotlin JVM package got this identical text.
const STALE_KOTLIN_SRC_DIR_BLOCK: &str = "    kotlin {\n      // The alef Kotlin backend emits binding sources at the project root\n      // (`packages/kotlin/`) rather than the Maven\n      // `src/main/kotlin/` convention. Pull them in explicitly so they end up\n      // in the compiled jar alongside any standard-layout sources.\n      srcDir(\".\")\n    }\n";

/// The stale `configure(KotlinJvm(...))` call [`scaffold_kotlin_jvm`] emitted before the fix
/// that added a trailing comma after the `KotlinJvm(...)` argument — ktlint enforces a trailing
/// comma after each multi-line `configure()` argument, so without it every `prek` run added the
/// comma back, which in turn made `alef sync-versions` regenerate the scaffold file to the
/// no-comma form: an endless formatter/emitter cycle.
const STALE_MAVEN_PUBLISHING_COMMA: &str = "      sourcesJar = true,\n    )\n  )\n";
const FIXED_MAVEN_PUBLISHING_COMMA: &str = "      sourcesJar = true,\n    ),\n  )\n";

/// Repair a pre-existing `packages/kotlin/build.gradle.kts` carrying either (or both) of the two
/// known-bad shapes above. `build.gradle.kts` is `generated_header: false` (create-only), so a
/// repo scaffolded before either fix keeps a build file that fails `publishToMavenCentral`
/// (Gradle 9 output-overlap validation from the stale `srcDir(".")`) or churns forever between
/// `prek`'s ktlint pass and `alef sync-versions` (the missing trailing comma) — not a
/// theoretical staleness, an actively broken publish/format loop. Each defect is repaired
/// independently via an exact substring match-and-replace against its known-bad text: neither
/// constant carries any per-project variable, so a match is unambiguous, and any consumer edit
/// inside either block (a reordered line, an added comment) fails the match and that half of the
/// repair is simply skipped rather than guessed at — the file's Gradle syntax elsewhere,
/// including any other hand customization, is never touched. Returns `false` (no-op, not an
/// error) when the file doesn't exist or neither known-bad shape is present. ~keep
pub(crate) fn migrate_kotlin_build_gradle(base_dir: &Path) -> anyhow::Result<bool> {
    let path = base_dir.join("packages/kotlin/build.gradle.kts");
    let Ok(existing) = std::fs::read_to_string(&path) else {
        return Ok(false);
    };

    let mut migrated = existing.clone();
    if migrated.matches(STALE_KOTLIN_SRC_DIR_BLOCK).count() == 1 {
        migrated = migrated.replacen(STALE_KOTLIN_SRC_DIR_BLOCK, "", 1);
    }
    if migrated.matches(STALE_MAVEN_PUBLISHING_COMMA).count() == 1 {
        migrated = migrated.replacen(STALE_MAVEN_PUBLISHING_COMMA, FIXED_MAVEN_PUBLISHING_COMMA, 1);
    }
    if migrated == existing {
        return Ok(false);
    }

    let parent = path.parent().context("build.gradle.kts path has no parent directory")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to create temporary file in {}", parent.display()))?;
    std::io::Write::write_all(&mut temporary, migrated.as_bytes())
        .with_context(|| format!("failed to write temporary file for {}", path.display()))?;
    temporary
        .persist(&path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    // Fires only after the replace above already succeeded: a completed self-heal, not an
    // outstanding problem. ~keep
    tracing::info!(
        path = %path.display(),
        "repaired pre-existing packages/kotlin/build.gradle.kts: dropped the stale root \
         kotlin srcDir(\".\") and/or added the missing mavenPublishing trailing comma"
    );
    Ok(true)
}

struct ScmUrls {
    connection: String,
    developer_connection: String,
}

fn scm_urls(repository: &str) -> ScmUrls {
    let normalized = repository.trim_end_matches(".git");
    let without_scheme = normalized
        .strip_prefix("https://")
        .or_else(|| normalized.strip_prefix("http://"))
        .unwrap_or(normalized);
    let (host, path) = without_scheme.split_once('/').unwrap_or((without_scheme, ""));
    let suffix = if path.is_empty() {
        String::new()
    } else {
        format!("/{path}.git")
    };

    ScmUrls {
        connection: format!("scm:git:git://{host}{suffix}"),
        developer_connection: format!("scm:git:ssh://git@{host}{suffix}"),
    }
}

fn kotlin_native_def(config: &ResolvedCrateConfig) -> String {
    format!(
        "headers = {}\nheaderFilter = {}_*\nlinkerOpts = -L../../../target/release -l{}\n",
        config.ffi_header_name(),
        config.ffi_prefix(),
        config.ffi_lib_name()
    )
}

fn scaffold_kotlin_native(_api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let project_name = format!("{}-native", config.name);
    let kotlin_plugin = maven::KOTLIN_JVM_PLUGIN;
    let crate_name = &config.name;
    let license_section = meta
        .license
        .as_deref()
        .map(|license| format!("\n## License\n\n{license}\n"))
        .unwrap_or_default();
    let readme = format!(
        r#"# {project_name}

{description}

## Building

```sh
cargo build --release -p {crate_name}-ffi
cd packages/kotlin-native
gradle build
```
"#,
        description = meta.description,
    ) + &license_section;
    let build_gradle = format!(
        r#"plugins {{
    kotlin("multiplatform") version "{kotlin_plugin}"
}}

kotlin {{
    linuxX64 {{
        compilations["main"].cinterops {{
            val {crate_name} by creating {{
                defFile = project.file("{crate_name}.def")
            }}
        }}
        binaries {{
            sharedLib()
        }}
    }}
}}
"#
    );

    Ok(vec![
        GeneratedFile {
            path: PathBuf::from("packages/kotlin-native/build.gradle.kts"),
            content: build_gradle,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/kotlin-native/settings.gradle.kts"),
            content: format!("rootProject.name = \"{project_name}\"\n"),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from(format!("packages/kotlin-native/{crate_name}.def")),
            content: kotlin_native_def(config),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/kotlin-native/.gitignore"),
            content: "build/\n.gradle/\n.idea/\n*.iml\n".to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/kotlin-native/README.md"),
            content: readme,
            generated_header: false,
        },
    ])
}

fn scaffold_kotlin_multiplatform(
    _api: &ApiSurface,
    config: &ResolvedCrateConfig,
) -> anyhow::Result<Vec<GeneratedFile>> {
    let meta = scaffold_meta(config);
    let project_name = format!("{}-kmp", config.name);
    let kotlin_plugin = maven::KOTLIN_JVM_PLUGIN;
    let crate_name = &config.name;
    let license_section = meta
        .license
        .as_deref()
        .map(|license| format!("\n## License\n\n{license}\n"))
        .unwrap_or_default();
    let readme = format!(
        r#"# {project_name}

{description}

## Building

```sh
cargo build --release -p {crate_name}-ffi
cd packages/kotlin-mpp
gradle build
```
"#,
        description = meta.description,
    ) + &license_section;
    let build_gradle = format!(
        r#"plugins {{
    kotlin("multiplatform") version "{kotlin_plugin}"
}}

kotlin {{
    jvm()

    linuxX64 {{
        compilations["main"].cinterops {{
            val {crate_name} by creating {{
                defFile = project.file("{crate_name}.def")
            }}
        }}
        binaries {{
            sharedLib()
        }}
    }}

    macosArm64 {{
        compilations["main"].cinterops {{
            val {crate_name} by creating {{
                defFile = project.file("{crate_name}.def")
            }}
        }}
        binaries {{
            sharedLib()
        }}
    }}
}}
"#
    );

    Ok(vec![
        GeneratedFile {
            path: PathBuf::from("packages/kotlin-mpp/build.gradle.kts"),
            content: build_gradle,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/kotlin-mpp/settings.gradle.kts"),
            content: format!("rootProject.name = \"{project_name}\"\n"),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from(format!("packages/kotlin-mpp/{crate_name}.def")),
            content: kotlin_native_def(config),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/kotlin-mpp/.gitignore"),
            content: "build/\n.gradle/\n.idea/\n*.iml\n".to_string(),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/kotlin-mpp/README.md"),
            content: readme,
            generated_header: false,
        },
    ])
}

#[cfg(test)]
mod migrate_tests {
    use super::*;

    fn pre_fix_build_gradle() -> String {
        format!(
            "sourceSets {{\n  main {{\n    java {{\n      srcDir(\"../java\")\n    }}\n{STALE_KOTLIN_SRC_DIR_BLOCK}  }}\n}}\n\nmavenPublishing {{\n  configure(\n    KotlinJvm(\n      javadocJar = JavadocJar.Empty(),\n{STALE_MAVEN_PUBLISHING_COMMA}\n  publishToMavenCentral()\n}}\n"
        )
    }

    #[test]
    fn should_repair_both_known_bad_shapes_independently() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg_dir = dir.path().join("packages/kotlin");
        std::fs::create_dir_all(&pkg_dir).expect("create packages/kotlin");
        std::fs::write(pkg_dir.join("build.gradle.kts"), pre_fix_build_gradle()).expect("write pre-fix file");

        let changed = migrate_kotlin_build_gradle(dir.path()).expect("migration must not error");
        assert!(
            changed,
            "a build.gradle.kts with both known-bad shapes must be reported as changed"
        );

        let on_disk = std::fs::read_to_string(pkg_dir.join("build.gradle.kts")).expect("read migrated file");
        assert!(
            !on_disk.contains("srcDir(\".\")"),
            "the stale root kotlin srcDir must be removed"
        );
        assert!(
            on_disk.contains("srcDir(\"../java\")"),
            "the java srcDir block must survive untouched"
        );
        assert!(
            on_disk.contains("sourcesJar = true,\n    ),\n  )"),
            "the trailing comma must be added after the KotlinJvm(...) argument"
        );

        let changed_again = migrate_kotlin_build_gradle(dir.path()).expect("second pass must not error");
        assert!(
            !changed_again,
            "second pass over an already-migrated file must be a no-op"
        );
    }

    #[test]
    fn should_repair_only_the_defect_that_is_present() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg_dir = dir.path().join("packages/kotlin");
        std::fs::create_dir_all(&pkg_dir).expect("create packages/kotlin");
        let only_comma_defect = format!(
            "mavenPublishing {{\n  configure(\n    KotlinJvm(\n      javadocJar = JavadocJar.Empty(),\n{STALE_MAVEN_PUBLISHING_COMMA}\n  publishToMavenCentral()\n}}\n"
        );
        std::fs::write(pkg_dir.join("build.gradle.kts"), &only_comma_defect).expect("write file");

        let changed = migrate_kotlin_build_gradle(dir.path()).expect("migration must not error");
        assert!(changed);

        let on_disk = std::fs::read_to_string(pkg_dir.join("build.gradle.kts")).expect("read migrated file");
        assert!(on_disk.contains("sourcesJar = true,\n    ),\n  )"));
    }

    #[test]
    fn should_not_touch_a_hand_edited_build_gradle() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pkg_dir = dir.path().join("packages/kotlin");
        std::fs::create_dir_all(&pkg_dir).expect("create packages/kotlin");
        let hand_written = "sourceSets {\n  main {\n    java {\n      srcDir(\"../java\")\n    }\n    kotlin {\n      // custom hand-written comment, not alef's\n      srcDir(\".\")\n    }\n  }\n}\n";
        std::fs::write(pkg_dir.join("build.gradle.kts"), hand_written).expect("write hand-edited file");

        let changed = migrate_kotlin_build_gradle(dir.path()).expect("migration must not error");
        assert!(
            !changed,
            "a hand-edited block must never match the exact known-bad text"
        );

        let on_disk = std::fs::read_to_string(pkg_dir.join("build.gradle.kts")).expect("read file");
        assert_eq!(
            on_disk, hand_written,
            "hand-edited build.gradle.kts must survive byte-for-byte"
        );
    }

    #[test]
    fn migrate_kotlin_build_gradle_is_a_no_op_when_file_does_not_exist() {
        let dir = tempfile::tempdir().expect("tempdir");
        let changed = migrate_kotlin_build_gradle(dir.path()).expect("must not error");
        assert!(!changed);
        assert!(!dir.path().join("packages/kotlin/build.gradle.kts").exists());
    }
}

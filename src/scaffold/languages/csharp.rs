use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::ApiSurface;
use crate::core::version::to_dotnet_assembly_version;
use crate::scaffold::naming::csharp_package_id;
use crate::{scaffold::scaffold_meta, scaffold::xml_escape};
use std::path::PathBuf;

/// Runtime Identifiers (RIDs) advertised by the binding NuGet package, each
/// paired with the Rust target triple that produces its native asset.
///
/// Every enabled platform must appear in `<RuntimeIdentifiers>` so the SDK packs
/// its `runtimes/<rid>/native/` payload into the NuGet artifact and consumers
/// resolve a matching native asset at restore time. Mismatches surface to
/// consumers as `warning CS8012: ... targets a different processor` followed by
/// `FileNotFoundException` at runtime — the exact failure mode observed on tslp
/// v1.9.0-rc.48. A RID whose triple is disabled via the workspace `[targets]`
/// opt-out table is dropped from the list.
pub const PUBLISHED_RUNTIME_IDENTIFIERS: &[(&str, &str)] = &[
    ("win-x64", "x86_64-pc-windows-msvc"),
    ("win-arm64", "aarch64-pc-windows-msvc"),
    ("linux-x64", "x86_64-unknown-linux-gnu"),
    ("linux-arm64", "aarch64-unknown-linux-gnu"),
    ("osx-x64", "x86_64-apple-darwin"),
    ("osx-arm64", "aarch64-apple-darwin"),
];

/// Render just the `.csproj` XML content for the given config and version string.
///
/// The produced csproj is designed to live at
/// `packages/csharp/<Namespace>/<Namespace>.csproj` and is a THIN meta-package:
/// - `../../../LICENSE` reaches the workspace root (3 path components deep)
/// - it carries only the managed assembly plus `runtime.json` (rendered from
///   `runtime.json.template` by CI before pack), which drives NuGet's RID-fallback
///   graph. Native closures ship separately in the per-RID
///   `<PackageId>.runtime.<rid>` packages (see the sibling `*.Runtime` project) —
///   packing `runtimes/**` directly into the meta blows past the NuGet package
///   size limit and returns HTTP 413.
///
/// This is exposed as a `pub` function so `alef-publish` can regenerate the
/// csproj before invoking `dotnet pack`, guaranteeing the metadata stays in
/// sync regardless of what is committed on disk.
pub fn render_csharp_csproj(config: &ResolvedCrateConfig, version: &str) -> String {
    let meta = scaffold_meta(config);
    let namespace = config.csharp_namespace();
    let package_id = csharp_package_id(config);

    let target_framework = config
        .csharp
        .as_ref()
        .and_then(|c| c.target_framework.clone())
        .unwrap_or_else(|| "net10.0".to_string());

    let authors_csproj = if meta.authors.is_empty() {
        String::new()
    } else {
        let escaped: Vec<String> = meta.authors.iter().map(|a| xml_escape(a)).collect();
        format!("    <Authors>{}</Authors>\n", escaped.join(";"))
    };
    let repository_csproj = meta
        .configured_repository
        .as_deref()
        .map(|repository| format!("    <RepositoryUrl>{}</RepositoryUrl>\n", xml_escape(repository)))
        .unwrap_or_default();

    let assembly_version = to_dotnet_assembly_version(version);
    let runtime_identifiers = PUBLISHED_RUNTIME_IDENTIFIERS
        .iter()
        .filter(|(_, triple)| config.target_enabled(triple))
        .map(|(rid, _)| *rid)
        .collect::<Vec<_>>()
        .join(";");

    format!(
        r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>{target_framework}</TargetFramework>
    <RootNamespace>{namespace}</RootNamespace>
    <PackageId>{package_id}</PackageId>
    <Version>{version}</Version>
    <AssemblyVersion>{assembly_version}</AssemblyVersion>
    <FileVersion>{assembly_version}</FileVersion>
    <InformationalVersion>{version}</InformationalVersion>
    <Description>{description}</Description>
    <PackageLicenseFile>LICENSE</PackageLicenseFile>
{repository}{authors}    <Company>Alef Team</Company>
    <Product>{namespace}</Product>
    <AllowUnsafeBlocks>true</AllowUnsafeBlocks>
    <Nullable>enable</Nullable>
    <!-- AnyCPU managed assembly so the PE `Machine` header stays processor-neutral
         and consumers never see `warning CS8012: ... targets a different processor`.
         Native asset resolution at consumer build time is driven by the
         `runtimes/<rid>/native/` payload baked into the NuGet package — not a
         single `<RuntimeIdentifier>` on the package author's side, which would
         force runtime-specific output and break AnyCPU packaging. -->
    <PlatformTarget>AnyCPU</PlatformTarget>
    <RuntimeIdentifiers>{runtime_identifiers}</RuntimeIdentifiers>
  </PropertyGroup>

  <ItemGroup>
    <None Include="../../../LICENSE" Pack="true" PackagePath="/" />
    <!-- Thin meta-package: no runtimes/** payload here. Native closures ship in the
         per-RID {package_id}.runtime.<rid> packages (see ../{namespace}.Runtime); this
         package only carries the managed assembly plus runtime.json, which tells
         NuGet's RID-fallback graph which runtime package to pull in for the
         consumer's RID. runtime.json is rendered
         from runtime.json.template by CI before pack; RequireRuntimeJson guards
         against packing without it. -->
    <None Include="runtime.json" Pack="true" PackagePath="/" Condition="Exists('runtime.json')" />
  </ItemGroup>

  <Target Name="RequireRuntimeJson" BeforeTargets="Pack">
    <Error Condition="!Exists('runtime.json')"
           Text="packages/csharp/{namespace}/runtime.json is missing. Render it from runtime.json.template (substituting {{{{VERSION}}}}) before packing {namespace}.csproj." />
  </Target>

  <ItemGroup>
    <Compile Include="../src/**/*.cs" />
  </ItemGroup>
{capsule_package_refs}</Project>
"#,
        target_framework = target_framework,
        namespace = namespace,
        package_id = package_id,
        version = version,
        assembly_version = assembly_version,
        runtime_identifiers = runtime_identifiers,
        description = meta.description,
        repository = repository_csproj,
        authors = authors_csproj,
        capsule_package_refs = capsule_package_refs(config),
    )
}

/// Render a `<PackageReference>` ItemGroup for host-native capsule (Language) passthrough
/// dependencies (e.g. NuGet `TreeSitter.DotNet`). Empty when no capsule types are configured.
fn capsule_package_refs(config: &ResolvedCrateConfig) -> String {
    let mut deps: Vec<(String, String)> = config
        .csharp
        .as_ref()
        .map(|c| {
            c.capsule_types
                .values()
                .filter(|cap| !cap.package.is_empty())
                .map(|cap| (cap.package.clone(), cap.package_version.clone()))
                .collect()
        })
        .unwrap_or_default();
    deps.sort();
    deps.dedup();
    if deps.is_empty() {
        return String::new();
    }
    let refs: String = deps
        .iter()
        .map(|(pkg, ver)| format!("    <PackageReference Include=\"{pkg}\" Version=\"{ver}\" />\n"))
        .collect();
    format!("\n  <ItemGroup>\n{refs}  </ItemGroup>\n")
}

/// Render the `runtime.json.template` for the C# thin meta-package.
///
/// This is a CI build input, not a shipped artifact: the publish pipeline substitutes
/// the release version for the literal `{{VERSION}}` token and writes `runtime.json`
/// beside the csproj before `dotnet pack`. The csproj's `RequireRuntimeJson` target
/// hard-errors when `runtime.json` is absent, so this template must exist for packing
/// to succeed. It drives NuGet's RID-fallback graph: each enabled published RID pulls
/// in its per-RID `<PackageId>.runtime.<rid>` native package, and musl RIDs fall back
/// to their glibc counterparts via `#import`.
pub fn render_csharp_runtime_json_template(config: &ResolvedCrateConfig) -> String {
    use serde_json::{Map, Value};

    let package_id = csharp_package_id(config);
    let enabled: Vec<&str> = PUBLISHED_RUNTIME_IDENTIFIERS
        .iter()
        .filter(|(_, triple)| config.target_enabled(triple))
        .map(|(rid, _)| *rid)
        .collect();

    let mut runtimes = Map::new();
    for rid in &enabled {
        let mut dependency = Map::new();
        dependency.insert(
            format!("{package_id}.runtime.{rid}"),
            Value::String("{{VERSION}}".to_string()),
        );
        let mut package = Map::new();
        package.insert(package_id.clone(), Value::Object(dependency));
        runtimes.insert((*rid).to_string(), Value::Object(package));
    }

    // musl RIDs ship no native package of their own; fall back to the glibc RID's
    // native asset via NuGet's `#import` chain. Only emit a fallback when its glibc
    // target is enabled so the import target always resolves.
    for (musl_rid, glibc_rid) in [("linux-musl-x64", "linux-x64"), ("linux-musl-arm64", "linux-arm64")] {
        if enabled.contains(&glibc_rid) {
            let mut import = Map::new();
            import.insert(
                "#import".to_string(),
                Value::Array(vec![Value::String(glibc_rid.to_string())]),
            );
            runtimes.insert(musl_rid.to_string(), Value::Object(import));
        }
    }

    let mut document = Map::new();
    document.insert("runtimes".to_string(), Value::Object(runtimes));
    let json = serde_json::to_string_pretty(&Value::Object(document)).expect("runtime.json template is serializable");
    format!("{json}\n")
}

/// Render the per-RID native runtime `.csproj` for the C# meta+runtime split.
///
/// The produced project lives at `packages/csharp/<Namespace>.Runtime/<Namespace>.Runtime.csproj`
/// and is the native-carrying half of the split the meta package's `runtime.json`
/// RID-fallback graph points at. It is a **native-only** package (no managed build
/// output) packed once per RID:
///
/// ```sh
/// dotnet pack <Namespace>.Runtime -p:PublishedRID=osx-arm64   # -> <PackageId>.runtime.osx-arm64
/// ```
///
/// CI loops the enabled RIDs, packing one `<PackageId>.runtime.<rid>` package each.
/// Native assets are read from the meta package's staging dir
/// (`../<Namespace>/runtimes/<rid>/native/**`) so the FFI libraries only need staging
/// once, exactly where `package_csharp` and the publish workflow already place them.
///
/// Exposed as `pub` so `alef-publish` can regenerate it before `dotnet pack`, keeping
/// the metadata in sync with `alef.toml` regardless of what is committed on disk.
pub fn render_csharp_runtime_csproj(config: &ResolvedCrateConfig, version: &str) -> String {
    let meta = scaffold_meta(config);
    let namespace = config.csharp_namespace();
    let package_id = csharp_package_id(config);

    let target_framework = config
        .csharp
        .as_ref()
        .and_then(|c| c.target_framework.clone())
        .unwrap_or_else(|| "net10.0".to_string());

    let authors_csproj = if meta.authors.is_empty() {
        String::new()
    } else {
        let escaped: Vec<String> = meta.authors.iter().map(|a| xml_escape(a)).collect();
        format!("    <Authors>{}</Authors>\n", escaped.join(";"))
    };
    let repository_csproj = meta
        .configured_repository
        .as_deref()
        .map(|repository| format!("    <RepositoryUrl>{}</RepositoryUrl>\n", xml_escape(repository)))
        .unwrap_or_default();

    let assembly_version = to_dotnet_assembly_version(version);

    // Default RID so a bare `dotnet pack` (no -p:PublishedRID) still evaluates the
    // PackageId and native glob; CI always overrides it per enabled RID. Pick the
    // first enabled published RID so the default resolves to a real target.
    let default_rid = PUBLISHED_RUNTIME_IDENTIFIERS
        .iter()
        .find(|(_, triple)| config.target_enabled(triple))
        .map(|(rid, _)| *rid)
        .unwrap_or("linux-x64");

    format!(
        r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>{target_framework}</TargetFramework>
    <!-- Native-only per-RID runtime package: the native half of the meta+runtime
         split. Packed once per RID via `dotnet pack -p:PublishedRID=<rid>`; CI loops
         the enabled RIDs. The meta package's runtime.json RID-fallback graph pulls in
         the matching {package_id}.runtime.<rid> at the consumer's restore. -->
    <PublishedRID Condition="'$(PublishedRID)' == ''">{default_rid}</PublishedRID>
    <PackageId>{package_id}.runtime.$(PublishedRID)</PackageId>
    <Version>{version}</Version>
    <AssemblyVersion>{assembly_version}</AssemblyVersion>
    <FileVersion>{assembly_version}</FileVersion>
    <InformationalVersion>{version}</InformationalVersion>
    <Description>{description} — native runtime assets for $(PublishedRID).</Description>
    <PackageLicenseFile>LICENSE</PackageLicenseFile>
{repository}{authors}    <Company>Alef Team</Company>
    <Product>{namespace}</Product>
    <!-- No managed assembly ships: this package carries only the RID's native
         payload under runtimes/<rid>/native/. Suppress the build output, its
         transitive dependencies, and the NU5128 "no lib/ref assembly for TFM"
         warning that a native-only package legitimately trips. -->
    <IncludeBuildOutput>false</IncludeBuildOutput>
    <IncludeSymbols>false</IncludeSymbols>
    <SuppressDependenciesWhenPacking>true</SuppressDependenciesWhenPacking>
    <EnableDefaultCompileItems>false</EnableDefaultCompileItems>
    <NoWarn>$(NoWarn);NU5128</NoWarn>
    <GenerateAssemblyInfo>false</GenerateAssemblyInfo>
  </PropertyGroup>

  <ItemGroup>
    <None Include="../../../LICENSE" Pack="true" PackagePath="/" />
    <None Include="../{namespace}/runtimes/$(PublishedRID)/native/**" Pack="true" PackagePath="runtimes/$(PublishedRID)/native/" />
  </ItemGroup>

  <Target Name="RequireRuntimeAssets" BeforeTargets="Pack">
    <ItemGroup>
      <_StagedNative Include="../{namespace}/runtimes/$(PublishedRID)/native/**" />
    </ItemGroup>
    <Error Condition="'@(_StagedNative)' == ''"
           Text="No native assets under packages/csharp/{namespace}/runtimes/$(PublishedRID)/native/; stage the FFI library for $(PublishedRID) before packing {namespace}.Runtime." />
  </Target>
</Project>
"#,
        target_framework = target_framework,
        namespace = namespace,
        package_id = package_id,
        version = version,
        assembly_version = assembly_version,
        default_rid = default_rid,
        description = meta.description,
        repository = repository_csproj,
        authors = authors_csproj,
    )
}

pub(crate) fn scaffold_csharp(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    let namespace = config.csharp_namespace();
    let content = render_csharp_csproj(config, &api.version);

    Ok(vec![
        GeneratedFile {
            // alef-publish stages the FFI shared libraries.  `../../../LICENSE` from that
            path: PathBuf::from(format!("packages/csharp/{0}/{0}.csproj", namespace)),
            content,
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from(format!("packages/csharp/{0}/runtime.json.template", namespace)),
            content: render_csharp_runtime_json_template(config),
            generated_header: false,
        },
        GeneratedFile {
            // Native half of the meta+runtime split, packed once per RID by CI.
            path: PathBuf::from(format!("packages/csharp/{0}.Runtime/{0}.Runtime.csproj", namespace)),
            content: render_csharp_runtime_csproj(config, &api.version),
            generated_header: false,
        },
        GeneratedFile {
            path: PathBuf::from("packages/csharp/.editorconfig"),
            content: "root = true\n\n[*.cs]\nindent_style = space\nindent_size = 4\nmax_line_length = 120\nend_of_line = lf\ncharset = utf-8\ntrim_trailing_whitespace = true\ninsert_final_newline = true\n".to_string(),
            generated_header: false,
        },
    ])
}

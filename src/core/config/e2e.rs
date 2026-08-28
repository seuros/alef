//! E2E test generation configuration types.

mod call;
mod defaults;
mod harness;
mod package;
mod root;
pub mod sample_url;
pub mod sample_url_manifest;
pub mod sample_url_template;
mod selection;
mod snippets;

#[cfg(test)]
mod raw_function_reads;
#[cfg(test)]
mod raw_result_var_reads;
#[cfg(test)]
mod tests;

pub use call::{ArgMapping, CallConfig, CallOverride, StreamingConfig, StreamingRecipe};
pub use harness::{HarnessConfig, HarnessOverride, RouteCallForm};
pub use package::{DependencyMode, HomebrewCliTest, PackageRef, RegistryConfig};
pub use root::E2eConfig;
pub use sample_url::{
    DEFAULT_DOCS_SAMPLE_BASE_URL, DocsSampleBaseUrl, InvalidSampleBaseUrl, SAMPLE_BASE_URL_CONFIG_KEY,
};
pub use sample_url_manifest::{
    InvalidSampleUrlManifest, SAMPLE_URL_MANIFEST_CONFIG_KEY, SampleUrlManifest, SampleUrlManifestConfig,
    merge_manifest_vars,
};
pub use sample_url_template::{
    InvalidSampleUrlTemplate, SAMPLE_URL_TEMPLATE_CONFIG_KEY, SAMPLE_URL_VARS_FIXTURE_KEY, SampleUrlTemplate,
    resolve_templated_sample_url,
};
pub use selection::SelectWhen;
pub use snippets::{SnippetCapabilities, SnippetConfig};

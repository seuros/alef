//! Java class-name derivations shared by the Java binding emitter and the API reference docs.
//!
//! The docs pages quote Java signatures verbatim (`throws <Exception>`), so the name they print
//! has to be the name the emitted `.java` file actually declares. Both sides call the functions
//! here rather than re-deriving a spelling from whatever string happens to be in scope -- the
//! docs previously built the throws clause from `[ffi] prefix`, which is a C symbol prefix and
//! is free to differ from the crate name the class is named after. ~keep

use crate::codegen::naming::to_class_name;

/// Name of the raw FFI wrapper class the Java backend declares for `crate_name`.
///
/// The `Rs` suffix keeps the raw FFI class distinct from the public facade class, which is this
/// name with the suffix stripped (`gen_bindings/mod.rs`'s `public_class`). Without it the facade
/// would delegate to itself and recurse forever. A crate whose name already ends in `Rs` keeps
/// the single suffix. ~keep
pub(crate) fn main_class_name(crate_name: &str) -> String {
    let base = to_class_name(&crate_name.replace('-', "_"));
    if base.ends_with("Rs") {
        base
    } else {
        format!("{base}Rs")
    }
}

/// Name of the checked exception class the Java backend declares for `crate_name`.
///
/// This is the class `exception_class.jinja` emits into `<MainClass>Exception.java` and the class
/// every generated Java method's `throws` clause names. ~keep
pub(crate) fn exception_class_name(crate_name: &str) -> String {
    format!("{}Exception", main_class_name(crate_name))
}

/// Name of the public facade class the Java backend declares for `crate_name`: [`main_class_name`]
/// with its `Rs` suffix stripped. This is the class `<PublicClass>.java` delegates to
/// [`main_class_name`]'s raw FFI wrapper -- see `gen_bindings/mod.rs`'s `public_class`. ~keep
pub(crate) fn public_class_name(crate_name: &str) -> String {
    main_class_name(crate_name).trim_end_matches("Rs").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_class_name_pascal_cases_a_hyphenated_crate_name() {
        assert_eq!(main_class_name("sample-multi-word"), "SampleMultiWordRs");
    }

    #[test]
    fn main_class_name_does_not_double_the_rs_suffix() {
        assert_eq!(main_class_name("sample_rs"), "SampleRs");
    }

    #[test]
    fn exception_class_name_appends_exception_to_the_main_class() {
        assert_eq!(exception_class_name("sample-multi-word"), "SampleMultiWordRsException");
    }

    #[test]
    fn public_class_name_strips_the_rs_suffix_from_the_main_class() {
        assert_eq!(public_class_name("sample-multi-word"), "SampleMultiWord");
    }

    #[test]
    fn public_class_name_strips_a_crate_name_whose_own_trailing_segment_is_rs() {
        assert_eq!(public_class_name("sample_rs"), "Sample");
    }

    /// `main_class_name` appends `Rs` when the crate name does not already end in it, so
    /// stripping that suffix here returns the PascalCased crate name unchanged -- the facade is
    /// named after the crate, and only the raw FFI class carries the marker. ~keep
    #[test]
    fn public_class_name_round_trips_a_crate_name_that_did_not_end_in_rs() {
        assert_eq!(main_class_name("sample-sensors"), "SampleSensorsRs");
        assert_eq!(public_class_name("sample-sensors"), "SampleSensors");
    }
}

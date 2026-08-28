//! Field-resolver fixtures shared by the Kotlin e2e tests.
//!
//! Split out of `tests.rs` to keep that file under its ratchet ceiling. ~keep

use crate::e2e::field_access::FieldResolver;
use std::collections::{HashMap, HashSet};

pub(super) fn make_resolver_for_finish_reason() -> FieldResolver {
    // Resolver for `choices[0].finish_reason` where:
    //   - `choices` is a registered array field (default index 0)
    //   - `choices.finish_reason` is optional (`@Nullable`)
    let mut optional = HashSet::new();
    optional.insert("choices.finish_reason".to_string());
    let mut arrays = HashSet::new();
    arrays.insert("choices".to_string());
    FieldResolver::new(&HashMap::new(), &optional, &HashSet::new(), &arrays, &HashSet::new())
}

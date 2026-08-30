use std::collections::{HashMap, HashSet};

use alef::e2e::field_access::PythonTypedDictMap;

#[test]
fn pre_0793_public_struct_literal_remains_source_compatible() {
    let map = PythonTypedDictMap {
        typeddict_types: HashSet::new(),
        field_types: HashMap::new(),
        root_type: Some("Report".to_string()),
    };

    assert_eq!(map.root_type.as_deref(), Some("Report"));
    assert!(map.is_empty());
}

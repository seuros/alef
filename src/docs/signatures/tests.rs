use super::*;
use crate::core::config::Language;
use crate::core::ir::{PrimitiveType, TypeRef};
use crate::docs::test_helpers::{TEST_CRATE_NAME, TEST_PREFIX, make_function, make_method, make_param};

mod ffi_handle_consistency;
mod ffi_status_code;
mod function_signatures;
mod identifier_gate;
mod method_signatures;

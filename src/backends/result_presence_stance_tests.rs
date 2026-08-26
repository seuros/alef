//! The cross-language control for the FFI result-presence channel.
//!
//! A Rust `Option<i64>` return crosses the C ABI as a bare `i64`: `None` and a legitimate
//! `Some(0)` become the same bits. The FFI backend answers that with an additive
//! `{fn}_has_result` companion export, and every backend that consumes the C ABI has to call it.
//!
//! The failure this file exists to prevent is not a wrong value — it is a *silent omission*. A
//! backend that never consults the companion does not error; it returns its target language's
//! zero wrapped as present, which is a perfectly plausible-looking result. Nothing in the
//! per-backend tests notices a backend that simply never mentions the channel, because a test
//! that is never written cannot fail. Only an exhaustive classification makes an omission
//! visible.
//!
//! [`presence_channel_stance`] is therefore an exhaustive match over [`Language`] with no `_`
//! arm: a new backend does not compile until someone states how it carries absence across the
//! boundary. That is the anti-drift half — an exclusion that is merely *implied* by a test which
//! never mentions a backend is exactly how a backend leaves a control unnoticed. ~keep

use crate::backends::ffi::type_map::result_presence_companion_exists;
use crate::core::config::Language;
use crate::core::ir::{PrimitiveType, ReceiverKind, TypeRef};

/// How a language's binding recovers "the `Option` return was `None`".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PresenceStance {
    /// Consumes the C ABI, so a scalar `Option` return is ambiguous and the binding must call the
    /// `{fn}_has_result` companion. Any backend here that stops calling it silently reports a
    /// zero as present.
    ConsumesCompanion,
    /// Consumes the C ABI but has not been wired to the companion yet. A known gap, named rather
    /// than left implicit, so it cannot be mistaken for a backend that was audited and cleared.
    ConsumesCabiNotYetWired,
    /// Hands a real Rust `Option<T>` to a macro framework (PyO3, napi-rs, magnus, ext-php-rs,
    /// wasm-bindgen, Rustler, extendr) or to its own non-C-ABI bridge, which lowers `None` to a
    /// genuine host null/nil/None. No sentinel is ever formed, so there is nothing to disambiguate.
    NativeOptional,
    /// Not a host binding surface: the C ABI producer itself, the Rust JNI shim emitter, and the
    /// docs/e2e-only targets.
    NoHostReturnSurface,
}

/// The classification. Deliberately an exhaustive match with no `_` arm. ~keep
const fn presence_channel_stance(language: Language) -> PresenceStance {
    match language {
        // Wired: the wrapper invokes `{fn}_has_result` before the primary call and reports
        // absence as the host's absent value.
        Language::Go | Language::Java | Language::Csharp | Language::Zig => PresenceStance::ConsumesCompanion,
        // Kotlin/JVM emits no downcall of its own — it calls the Java facade and applies
        // `.orElse(null)`, so it consumes the companion transitively through Java.
        Language::Kotlin => PresenceStance::ConsumesCompanion,
        // Audited and confirmed broken; the companion exists but this backend does not call it.
        // Dart's `ffi` style is broken one layer below the presence channel: its `dart:ffi`
        // typedef declares `Pointer<Void>` where the FFI crate returns `int64_t`, so a gate over
        // it would guard a call of the wrong width.
        Language::Dart => PresenceStance::ConsumesCabiNotYetWired,
        // Real `Option<T>` into a macro framework: Python `None`, JS `null`, Ruby `nil`, PHP
        // `null`, Elixir `nil`, R `NULL`/`NA`.
        Language::Python | Language::Node | Language::Ruby | Language::Php | Language::Elixir | Language::R => {
            PresenceStance::NativeOptional
        }
        // WASM lowers through wasm-bindgen to `undefined`. Gleam emits no Rust at all — it
        // declares Erlang externals against the Rustler NIF module. Swift routes *every*
        // optional through a serde_json string bridge (swift-bridge cannot express a custom
        // `Option` result), so `None` travels as literal JSON `null` and decodes to `Int64?`.
        Language::Wasm | Language::Gleam | Language::Swift => PresenceStance::NativeOptional,
        // KotlinAndroid forces the JNI style and delegates its emitters to the kotlin backend;
        // both reach the core crate through the `Java_*` Rust shim, whose nullable-jstring/JSON
        // channel already separates `None` from `Some(0)`.
        Language::KotlinAndroid | Language::Jni => PresenceStance::NativeOptional,
        // Ffi is the C ABI producer — it *exports* the companion rather than consuming one.
        // Rust and C are docs/e2e targets with no generated host return surface.
        Language::Ffi | Language::Rust | Language::C => PresenceStance::NoHostReturnSurface,
    }
}

/// Every `Language` is classified, and the enum's own `ALL` list is the source of the roster so a
/// new variant cannot be added without landing here too.
#[test]
fn every_language_states_a_presence_stance() {
    for language in Language::ALL {
        let _ = presence_channel_stance(language);
    }
    assert_eq!(
        Language::ALL.len(),
        20,
        "a Language variant was added or removed; give it a presence stance above"
    );
}

/// The gap list is a ledger, not a wildcard. When a backend is wired, its arm moves to
/// `ConsumesCompanion` and this test is what forces the ledger to be updated rather than left
/// stale — a backend silently remaining "not yet wired" after being fixed is how a real control
/// decays into a comment. ~keep
#[test]
fn the_unwired_c_abi_backends_are_exactly_the_known_gaps() {
    let unwired: Vec<Language> = Language::ALL
        .into_iter()
        .filter(|language| presence_channel_stance(*language) == PresenceStance::ConsumesCabiNotYetWired)
        .collect();

    assert_eq!(
        unwired,
        vec![Language::Dart],
        "the set of C-ABI backends still missing the presence companion changed"
    );
}

/// The authority's own contract, restated at the level this file reasons about: the companion is
/// what makes a scalar `Option` return recoverable, and it deliberately does not exist for an
/// owned receiver, whose first call already removed the handle from the registry.
#[test]
fn the_companion_covers_scalar_optionals_except_on_an_owned_receiver() {
    let scalar_option = TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::I64)));

    assert!(result_presence_companion_exists(&scalar_option, None));
    assert!(result_presence_companion_exists(
        &scalar_option,
        Some(&ReceiverKind::Ref)
    ));
    assert!(result_presence_companion_exists(
        &scalar_option,
        Some(&ReceiverKind::RefMut)
    ));
    assert!(
        !result_presence_companion_exists(&scalar_option, Some(&ReceiverKind::Owned)),
        "an owned receiver's first call consumes the handle, so the companion cannot re-invoke it"
    );

    assert!(
        !result_presence_companion_exists(&TypeRef::Optional(Box::new(TypeRef::String)), None),
        "`Option<String>` already carries a real null pointer and needs no companion"
    );
    assert!(
        !result_presence_companion_exists(&TypeRef::Primitive(PrimitiveType::I64), None),
        "a non-optional return has no absence to report"
    );
}

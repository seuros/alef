use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::TypeDef;
use ahash::AHashSet;

/// The type names the generated PyO3 module emits no `#[pyclass]` for.
///
/// Every pass that writes a generated binding class name -- the `#[pyclass]` emitter itself, the
/// visitor trait bridge that constructs one, and the `.pyi` protocol stub that annotates a
/// parameter with one -- must read the same answer, or one pass names a class another pass never
/// wrote. The `#[pyclass]` emitter is the authority, so this reproduces exactly the removals it
/// applies:
///
/// - `[crates.python] exclude_types`, which no IR flag records;
/// - `TypeDef::binding_excluded`, the IR-level removal;
/// - `[crates.python] capsule_types`, whose entries travel as raw `PyCapsule` handles and are
///   skipped by the emitter outright;
/// - the capsule/opaque intersection [`super::config_opaque::exclude_capsule_opaque_types`]
///   already owns, kept as a call so that rule keeps one definition.
///
/// `api.excluded_type_paths` is deliberately absent: those types are not in `api.types` at all, so
/// callers that walk the surface never reach them, and callers that resolve a name by lookup must
/// check that map themselves.
///
/// Takes the type slice rather than an `ApiSurface` because the Python e2e generator holds only
/// `&[TypeDef]` and must reach the same answer as the two binding generators. ~keep
pub(crate) fn pyclass_absent_type_names(config: &ResolvedCrateConfig, types: &[TypeDef]) -> AHashSet<String> {
    let mut absent: AHashSet<String> = config
        .python
        .as_ref()
        .map(|python| python.exclude_types.iter().cloned().collect())
        .unwrap_or_default();
    absent.extend(
        types
            .iter()
            .filter(|typ| typ.binding_excluded)
            .map(|typ| typ.name.clone()),
    );

    let capsule_types = config
        .python
        .as_ref()
        .map(|python| python.capsule_types.clone())
        .unwrap_or_default();
    super::config_opaque::exclude_capsule_opaque_types(&mut absent, config, &capsule_types);
    absent.extend(capsule_types.keys().cloned());

    absent
}

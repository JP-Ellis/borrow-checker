//! Importer registry types for the BorrowChecker import pipeline.
//!
//! This module provides [`ImporterFactory`], a lightweight `Clone` descriptor
//! that associates a stable format name with two static function pointers: one for
//! format detection and one for constructing a boxed [`crate::Importer`].
//!
//! Plugins register factories at startup; the core engine iterates them to detect
//! and drive imports without taking ownership of any concrete importer type.

use crate::Importer;

/// A lightweight descriptor for a single importer format.
///
/// Each `ImporterFactory` bundles a stable format name with two static function
/// pointers: one for sniffing whether a byte slice looks like the format, and one
/// for constructing a fresh [`Box<dyn Importer>`].
///
/// External code must use [`ImporterFactory::new`] to construct instances; the
/// struct's private fields prevent struct-literal construction outside this crate.
/// The `#[non_exhaustive]` attribute is retained for consistency with the project's
/// clippy settings.
///
/// # Example
///
/// ```rust
/// use bc_core::{ImportConfig, ImportError, Importer, ImporterFactory, RawTransaction};
///
/// struct MyImporter;
///
/// impl Importer for MyImporter {
///     fn name(&self) -> &'static str { "my-format" }
///     fn detect(&self, _bytes: &[u8]) -> bool { true }
///     fn import(
///         &self,
///         _bytes: &[u8],
///         _config: &ImportConfig,
///     ) -> Result<Vec<RawTransaction>, ImportError> {
///         Ok(vec![])
///     }
/// }
///
/// fn detect_my(_b: &[u8]) -> bool { true }
/// fn create_my() -> Box<dyn Importer> { Box::new(MyImporter) }
///
/// let factory = ImporterFactory::new("my-format", detect_my, create_my);
/// assert_eq!(factory.name(), "my-format");
/// assert!(factory.detect(b"anything"));
/// assert_eq!(factory.create().name(), "my-format");
/// ```
#[non_exhaustive]
#[derive(Clone)]
pub struct ImporterFactory {
    /// Stable format identifier (e.g. `"csv"`, `"ofx"`).
    name: &'static str,
    /// Static function pointer used to sniff whether bytes match this format.
    detect: fn(&[u8]) -> bool,
    /// Static function pointer that constructs a fresh boxed importer.
    create: fn() -> Box<dyn Importer>,
}

impl core::fmt::Debug for ImporterFactory {
    /// Formats the factory, showing only the `name` field.
    ///
    /// Function pointer addresses are unstable and convey no useful information,
    /// so only `name` is included in the debug output.
    #[inline]
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ImporterFactory")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// A collection of [`ImporterFactory`] instances that provides format auto-detection
/// and importer creation for the BorrowChecker import pipeline.
///
/// `ImporterRegistry` maintains an ordered list of factories. When detecting a format,
/// the first factory whose `detect` function returns `true` wins. Factories are
/// iterated in insertion order, so registration order determines detection priority.
///
/// # Example
///
/// ```rust
/// use bc_core::{ImportConfig, ImportError, Importer, ImporterFactory, ImporterRegistry, RawTransaction};
///
/// struct MyImporter;
///
/// impl Importer for MyImporter {
///     fn name(&self) -> &'static str { "my-format" }
///     fn detect(&self, _bytes: &[u8]) -> bool { bytes.starts_with(b"MY") }
///     fn import(
///         &self,
///         _bytes: &[u8],
///         _config: &ImportConfig,
///     ) -> Result<Vec<RawTransaction>, ImportError> {
///         Ok(vec![])
///     }
/// }
///
/// fn detect_my(b: &[u8]) -> bool { b.starts_with(b"MY") }
/// fn create_my() -> Box<dyn Importer> { Box::new(MyImporter) }
///
/// let mut registry = ImporterRegistry::new();
/// registry.register(ImporterFactory::new("my-format", detect_my, create_my));
/// assert_eq!(registry.detect_format(b"MY data"), Some("my-format"));
/// ```
#[expect(
    clippy::module_name_repetitions,
    reason = "ImporterRegistry is the canonical public name; renaming to Registry would be ambiguous at the crate root"
)]
#[non_exhaustive]
#[derive(Debug, Default)]
pub struct ImporterRegistry {
    /// The ordered list of registered importer factories.
    factories: Vec<ImporterFactory>,
}

impl ImporterRegistry {
    /// Creates an empty [`ImporterRegistry`].
    ///
    /// # Returns
    ///
    /// A new, empty `ImporterRegistry` with no registered factories.
    ///
    /// # Example
    ///
    /// ```rust
    /// use bc_core::ImporterRegistry;
    ///
    /// let registry = ImporterRegistry::new();
    /// ```
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a factory, returning `&mut self` for method chaining.
    ///
    /// Factories are stored in insertion order, which determines detection priority
    /// when multiple factories could match the same input.
    ///
    /// # Arguments
    ///
    /// * `factory` - The [`ImporterFactory`] to register.
    ///
    /// # Returns
    ///
    /// `&mut self` to allow method chaining.
    ///
    /// # Example
    ///
    /// ```rust
    /// use bc_core::{Importer, ImporterFactory, ImporterRegistry, ImportConfig, ImportError, RawTransaction};
    ///
    /// struct Stub;
    /// impl Importer for Stub {
    ///     fn name(&self) -> &'static str { "stub" }
    ///     fn detect(&self, _: &[u8]) -> bool { false }
    ///     fn import(&self, _: &[u8], _: &ImportConfig) -> Result<Vec<RawTransaction>, ImportError> { Ok(vec![]) }
    /// }
    /// fn detect_stub(_: &[u8]) -> bool { false }
    /// fn create_stub() -> Box<dyn Importer> { Box::new(Stub) }
    ///
    /// let mut registry = ImporterRegistry::new();
    /// registry
    ///     .register(ImporterFactory::new("a", detect_stub, create_stub))
    ///     .register(ImporterFactory::new("b", detect_stub, create_stub));
    /// ```
    #[inline]
    pub fn register(&mut self, factory: ImporterFactory) -> &mut Self {
        self.factories.push(factory);
        self
    }

    /// Returns the name of the first format whose `detect` function returns `true`, or `None`.
    ///
    /// Factories are checked in insertion order; the first match wins.
    ///
    /// # Arguments
    ///
    /// * `bytes` - Raw file bytes to inspect.
    ///
    /// # Returns
    ///
    /// `Some(&'static str)` with the format name if a match is found, or `None`.
    #[inline]
    #[must_use]
    pub fn detect_format(&self, bytes: &[u8]) -> Option<&'static str> {
        self.factories
            .iter()
            .find(|f| f.detect(bytes))
            .map(ImporterFactory::name)
    }

    /// Creates an importer for the named format, or `None` if not registered.
    ///
    /// Performs a linear scan over the registered factories and returns an importer
    /// from the first factory whose name matches exactly.
    ///
    /// # Arguments
    ///
    /// * `name` - The format name to look up (e.g. `"csv"`, `"ofx"`).
    ///
    /// # Returns
    ///
    /// `Some(Box<dyn Importer>)` if the format is registered, or `None`.
    #[inline]
    #[must_use]
    pub fn create_for_name(&self, name: &str) -> Option<Box<dyn crate::Importer>> {
        self.factories
            .iter()
            .find(|f| f.name() == name)
            .map(ImporterFactory::create)
    }

    /// Creates an importer for the first format that detects `bytes`, or `None`.
    ///
    /// Factories are checked in insertion order; the first match wins.
    ///
    /// # Arguments
    ///
    /// * `bytes` - Raw file bytes to inspect.
    ///
    /// # Returns
    ///
    /// `Some(Box<dyn Importer>)` if a matching format is found, or `None`.
    #[inline]
    #[must_use]
    pub fn create_for_bytes(&self, bytes: &[u8]) -> Option<Box<dyn crate::Importer>> {
        self.factories
            .iter()
            .find(|f| f.detect(bytes))
            .map(ImporterFactory::create)
    }

    /// Returns an iterator over registered format names, in insertion order.
    ///
    /// # Returns
    ///
    /// An iterator yielding `&'static str` format names.
    ///
    /// # Example
    ///
    /// ```rust
    /// use bc_core::{Importer, ImporterFactory, ImporterRegistry, ImportConfig, ImportError, RawTransaction};
    ///
    /// struct Stub;
    /// impl Importer for Stub {
    ///     fn name(&self) -> &'static str { "stub" }
    ///     fn detect(&self, _: &[u8]) -> bool { false }
    ///     fn import(&self, _: &[u8], _: &ImportConfig) -> Result<Vec<RawTransaction>, ImportError> { Ok(vec![]) }
    /// }
    /// fn detect_stub(_: &[u8]) -> bool { false }
    /// fn create_stub() -> Box<dyn Importer> { Box::new(Stub) }
    ///
    /// let mut registry = ImporterRegistry::new();
    /// registry
    ///     .register(ImporterFactory::new("csv", detect_stub, create_stub))
    ///     .register(ImporterFactory::new("ofx", detect_stub, create_stub));
    /// let names: Vec<_> = registry.names().collect();
    /// assert_eq!(names, &["csv", "ofx"]);
    /// ```
    #[inline]
    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.factories.iter().map(ImporterFactory::name)
    }
}

impl ImporterFactory {
    /// Constructs a new [`ImporterFactory`].
    ///
    /// # Arguments
    ///
    /// * `name` - A stable, short identifier for the format (e.g. `"csv"`).
    /// * `detect` - A static function that returns `true` when the given bytes
    ///   look like this format.
    /// * `create` - A static function that constructs and returns a fresh
    ///   [`Box<dyn Importer>`] for this format.
    ///
    /// # Returns
    ///
    /// A new [`ImporterFactory`] wrapping the provided name and function pointers.
    ///
    /// # Example
    ///
    /// ```rust
    /// use bc_core::{ImportConfig, ImportError, Importer, ImporterFactory, RawTransaction};
    ///
    /// struct NullImporter;
    /// impl Importer for NullImporter {
    ///     fn name(&self) -> &'static str { "null" }
    ///     fn detect(&self, _bytes: &[u8]) -> bool { false }
    ///     fn import(
    ///         &self,
    ///         _bytes: &[u8],
    ///         _config: &ImportConfig,
    ///     ) -> Result<Vec<RawTransaction>, ImportError> {
    ///         Ok(vec![])
    ///     }
    /// }
    ///
    /// fn detect_null(_b: &[u8]) -> bool { false }
    /// fn create_null() -> Box<dyn Importer> { Box::new(NullImporter) }
    ///
    /// let factory = ImporterFactory::new("null", detect_null, create_null);
    /// assert_eq!(factory.name(), "null");
    /// ```
    #[inline]
    #[must_use]
    pub fn new(
        name: &'static str,
        detect: fn(&[u8]) -> bool,
        create: fn() -> Box<dyn Importer>,
    ) -> Self {
        Self {
            name,
            detect,
            create,
        }
    }

    /// Returns the stable format identifier for this factory.
    ///
    /// # Returns
    ///
    /// The `name` supplied at construction time (e.g. `"csv"`, `"ofx"`).
    #[inline]
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Returns `true` if `bytes` look like input this format can handle.
    ///
    /// Delegates to the `detect` function pointer supplied at construction time.
    /// Implementations are expected to be fast and non-panicking.
    ///
    /// # Arguments
    ///
    /// * `bytes` - Raw file bytes to inspect.
    ///
    /// # Returns
    ///
    /// `true` if the bytes appear to be in this format, `false` otherwise.
    #[inline]
    #[must_use]
    pub fn detect(&self, bytes: &[u8]) -> bool {
        (self.detect)(bytes)
    }

    /// Constructs and returns a fresh boxed importer for this format.
    ///
    /// Delegates to the `create` function pointer supplied at construction time.
    /// Each call produces an independent importer instance.
    ///
    /// # Returns
    ///
    /// A [`Box<dyn Importer>`] ready for use.
    #[inline]
    #[must_use]
    pub fn create(&self) -> Box<dyn Importer> {
        (self.create)()
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    struct StubImporter;

    impl crate::Importer for StubImporter {
        fn name(&self) -> &'static str {
            "stub"
        }

        fn detect(&self, _bytes: &[u8]) -> bool {
            true
        }

        fn import(
            &self,
            _bytes: &[u8],
            _config: &crate::ImportConfig,
        ) -> Result<Vec<crate::RawTransaction>, crate::ImportError> {
            Ok(vec![])
        }
    }

    fn detect_stub(_b: &[u8]) -> bool {
        true
    }

    fn create_stub() -> Box<dyn crate::Importer> {
        Box::new(StubImporter)
    }

    #[test]
    fn factory_name_returns_registered_name() {
        let f = ImporterFactory::new("stub", detect_stub, create_stub);
        assert_eq!(f.name(), "stub");
    }

    #[test]
    fn factory_detect_delegates_to_fn_pointer() {
        let f = ImporterFactory::new("stub", detect_stub, create_stub);
        assert!(f.detect(b"anything"));
    }

    #[test]
    fn factory_create_returns_importer_with_correct_name() {
        let f = ImporterFactory::new("stub", detect_stub, create_stub);
        let imp = f.create();
        assert_eq!(imp.name(), "stub");
    }

    #[test]
    fn factory_detect_returns_false_when_fn_pointer_returns_false() {
        fn never(_b: &[u8]) -> bool {
            false
        }
        let f = ImporterFactory::new("stub", never, create_stub);
        assert!(!f.detect(b"anything"));
    }

    // ── ImporterRegistry tests ──────────────────────────────────────────────────

    #[test]
    fn registry_detect_format_returns_name_of_matching_factory() {
        let mut reg = ImporterRegistry::new();
        reg.register(ImporterFactory::new("stub", detect_stub, create_stub));
        assert_eq!(reg.detect_format(b"hello"), Some("stub"));
    }

    #[test]
    fn registry_detect_format_returns_none_when_nothing_matches() {
        let mut reg = ImporterRegistry::new();
        reg.register(ImporterFactory::new("stub", |_| false, create_stub));
        assert_eq!(reg.detect_format(b"hello"), None);
    }

    #[test]
    fn registry_detect_format_first_match_wins() {
        fn always(_: &[u8]) -> bool {
            true
        }
        fn create2() -> Box<dyn crate::Importer> {
            Box::new(StubImporter)
        }
        let mut reg = ImporterRegistry::new();
        reg.register(ImporterFactory::new("first", always, create_stub));
        reg.register(ImporterFactory::new("second", always, create2));
        assert_eq!(reg.detect_format(b"x"), Some("first"));
    }

    #[test]
    fn registry_create_for_name_returns_correct_importer() {
        let mut reg = ImporterRegistry::new();
        reg.register(ImporterFactory::new("stub", detect_stub, create_stub));
        let imp = reg
            .create_for_name("stub")
            .expect("stub should be registered");
        assert_eq!(imp.name(), "stub");
    }

    #[test]
    fn registry_create_for_name_returns_none_for_unknown_format() {
        let reg = ImporterRegistry::new();
        assert!(reg.create_for_name("unknown").is_none());
    }

    #[test]
    fn registry_create_for_bytes_returns_importer_when_detected() {
        let mut reg = ImporterRegistry::new();
        reg.register(ImporterFactory::new("stub", detect_stub, create_stub));
        let imp = reg.create_for_bytes(b"hello").expect("should detect stub");
        assert_eq!(imp.name(), "stub");
    }

    #[test]
    fn registry_create_for_bytes_returns_none_when_no_match() {
        let mut reg = ImporterRegistry::new();
        reg.register(ImporterFactory::new("stub", |_| false, create_stub));
        assert!(reg.create_for_bytes(b"hello").is_none());
    }

    #[test]
    fn registry_names_iterates_in_insertion_order() {
        let mut reg = ImporterRegistry::new();
        reg.register(ImporterFactory::new("csv", |_| false, create_stub))
            .register(ImporterFactory::new("ofx", |_| false, create_stub));
        let names: Vec<_> = reg.names().collect();
        assert_eq!(names, &["csv", "ofx"]);
    }

    #[test]
    fn registry_register_is_chainable() {
        let mut reg = ImporterRegistry::new();
        reg.register(ImporterFactory::new("a", |_| false, create_stub))
            .register(ImporterFactory::new("b", |_| false, create_stub));
        let names: Vec<_> = reg.names().collect();
        assert_eq!(names, &["a", "b"]);
    }

    #[test]
    fn factory_create_returns_distinct_instance_each_call() {
        use pretty_assertions::assert_ne;
        // Use a non-ZST importer so the allocator returns distinct addresses for
        // each heap allocation (ZSTs may share a sentinel address).
        #[expect(dead_code, reason = "field exists solely to make the type non-ZST")]
        struct SizedStub(u8);
        impl crate::Importer for SizedStub {
            fn name(&self) -> &'static str {
                "sized-stub"
            }

            fn detect(&self, _bytes: &[u8]) -> bool {
                true
            }

            fn import(
                &self,
                _bytes: &[u8],
                _config: &crate::ImportConfig,
            ) -> Result<Vec<crate::RawTransaction>, crate::ImportError> {
                Ok(vec![])
            }
        }
        fn create_sized() -> Box<dyn crate::Importer> {
            Box::new(SizedStub(0))
        }
        let f = ImporterFactory::new("sized-stub", detect_stub, create_sized);
        let a = f.create();
        let b = f.create();
        // Each call must produce a new allocation — raw pointers must differ.
        let a_ptr = core::ptr::from_ref::<dyn crate::Importer>(&*a).cast::<u8>();
        let b_ptr = core::ptr::from_ref::<dyn crate::Importer>(&*b).cast::<u8>();
        assert_ne!(a_ptr, b_ptr);
    }
}

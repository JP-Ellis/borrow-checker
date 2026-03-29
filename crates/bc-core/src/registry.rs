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

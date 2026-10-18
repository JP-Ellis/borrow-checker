//! Proc-macro support for `bc-sdk`.
//!
//! Provides the `#[importer]` attribute macro that generates WIT export glue
//! for types implementing `bc_sdk::Importer`, and optionally `bc_sdk::Exporter`.

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::Ident;
use syn::ItemImpl;
use syn::parse_macro_input;
use syn::parse2;

/// Generates WASM Component Model export glue for an `impl Importer` block.
///
/// Apply this attribute to an `impl bc_sdk::Importer for YourType` block.
/// The implementing type must also implement [`Default`].
///
/// The world also requires an exporter. Plain `#[importer]` exports one that
/// answers `ExportError::Unsupported`. `#[importer(exporter)]` exports one
/// that calls `<YourType as bc_sdk::Exporter>::export`, so the type must
/// implement that trait as well.
///
/// # Example
///
/// ```rust,ignore
/// use bc_sdk::{ImportConfig, ImportError, ImportOutput, Importer};
///
/// #[derive(Default)]
/// struct CsvImporter;
///
/// #[bc_sdk::importer]
/// impl Importer for CsvImporter {
///     fn name(&self) -> &str { "csv" }
///     fn import(&self, config: ImportConfig) -> Result<ImportOutput, ImportError> {
///         Ok(ImportOutput::default())
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn importer(attr: TokenStream, item: TokenStream) -> TokenStream {
    let item_impl = parse_macro_input!(item as ItemImpl);
    let glue = match parse_attr(attr.into()) {
        Ok(glue) => glue,
        Err(e) => return e.to_compile_error().into(),
    };
    match generate_importer_export(&item_impl, glue) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// What the generated exporter export does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExporterGlue {
    /// Answer every export with `ExportError::Unsupported`.
    Unsupported,
    /// Call `<T as bc_sdk::Exporter>::export`.
    Delegate,
}

/// Reads the attribute argument: nothing, or `exporter`.
///
/// # Errors
///
/// Returns a [`syn::Error`] for any other argument.
fn parse_attr(attr: TokenStream2) -> syn::Result<ExporterGlue> {
    if attr.is_empty() {
        return Ok(ExporterGlue::Unsupported);
    }
    let flag: Ident = parse2(attr)?;
    if flag == "exporter" {
        Ok(ExporterGlue::Delegate)
    } else {
        Err(syn::Error::new_spanned(
            flag,
            "unknown argument; the only accepted argument is `exporter`",
        ))
    }
}

/// Generates the token stream for WIT export glue from an `impl Importer` block.
///
/// # Errors
///
/// Returns a [`syn::Error`] if the input is not a trait impl block.
fn generate_importer_export(item_impl: &ItemImpl, glue: ExporterGlue) -> syn::Result<TokenStream2> {
    let self_ty = &item_impl.self_ty;

    if item_impl.trait_.is_none() {
        return Err(syn::Error::new_spanned(
            item_impl,
            "#[importer] must be applied to `impl Importer for YourType`",
        ));
    }

    let export_struct = quote! { __BcImporterExport };

    let export_body = match glue {
        ExporterGlue::Unsupported => quote! {
            fn export(
                _snapshot: ::bc_sdk::__bindings::exports::borrow_checker::sdk::exporter::ExportSnapshot,
            ) -> ::std::result::Result<
                ::std::vec::Vec<
                    ::bc_sdk::__bindings::exports::borrow_checker::sdk::exporter::ExportFile,
                >,
                ::bc_sdk::__bindings::exports::borrow_checker::sdk::exporter::ExportError,
            > {
                ::std::result::Result::Err(
                    ::bc_sdk::__bindings::exports::borrow_checker::sdk::exporter::ExportError::Unsupported,
                )
            }
        },
        ExporterGlue::Delegate => quote! {
            fn export(
                snapshot: ::bc_sdk::__bindings::exports::borrow_checker::sdk::exporter::ExportSnapshot,
            ) -> ::std::result::Result<
                ::std::vec::Vec<
                    ::bc_sdk::__bindings::exports::borrow_checker::sdk::exporter::ExportFile,
                >,
                ::bc_sdk::__bindings::exports::borrow_checker::sdk::exporter::ExportError,
            > {
                let snapshot = ::bc_sdk::ExportSnapshot::try_from(snapshot)
                    .map_err(::bc_sdk::__bindings::exports::borrow_checker::sdk::exporter::ExportError::from)?;
                <#self_ty as ::bc_sdk::Exporter>::export(
                    &<#self_ty as ::std::default::Default>::default(),
                    snapshot,
                )
                .map(|files| files.into_iter().map(::core::convert::Into::into).collect())
                .map_err(::core::convert::Into::into)
            }
        },
    };

    let expanded = quote! {
        #item_impl

        #[doc(hidden)]
        struct #export_struct;

        impl ::bc_sdk::__bindings::exports::borrow_checker::sdk::importer::Guest
            for #export_struct
        {
            fn sdk_abi() -> u32 {
                ::bc_sdk::SDK_ABI
            }

            fn name() -> ::std::string::String {
                <#self_ty as ::bc_sdk::Importer>::name(
                    &<#self_ty as ::std::default::Default>::default()
                ).to_owned()
            }

            fn parse(
                config: ::std::string::String,
            ) -> ::std::result::Result<
                ::bc_sdk::__bindings::exports::borrow_checker::sdk::importer::ImportOutput,
                ::bc_sdk::__bindings::exports::borrow_checker::sdk::importer::ImportError,
            > {
                let config = ::bc_sdk::ImportConfig::from_json_string(config);
                <#self_ty as ::bc_sdk::Importer>::import(
                    &<#self_ty as ::std::default::Default>::default(),
                    config,
                )
                .map(::core::convert::Into::into)
                .map_err(::core::convert::Into::into)
            }

            fn validate(
                config: ::std::string::String,
            ) -> ::std::result::Result<
                (),
                ::bc_sdk::__bindings::exports::borrow_checker::sdk::importer::ImportError,
            > {
                let config = ::bc_sdk::ImportConfig::from_json_string(config);
                <#self_ty as ::bc_sdk::Importer>::validate(
                    &<#self_ty as ::std::default::Default>::default(),
                    config,
                )
                .map_err(::core::convert::Into::into)
            }
        }

        impl ::bc_sdk::__bindings::exports::borrow_checker::sdk::exporter::Guest
            for #export_struct
        {
            #export_body
        }

        ::bc_sdk::export!(#export_struct with_types_in ::bc_sdk::__bindings);
    };

    Ok(expanded)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use syn::parse_str;

    use super::*;

    fn sample_impl() -> ItemImpl {
        parse_str(
            "impl Importer for MyPlugin {
                fn name(&self) -> &str { \"my-plugin\" }
                fn import(&self, _: ImportConfig)
                    -> Result<ImportOutput, ImportError> { Ok(ImportOutput::default()) }
                fn validate(&self, _: ImportConfig) -> Result<(), ImportError> { Ok(()) }
            }",
        )
        .expect("test input is valid syn")
    }

    #[test]
    fn trait_impl_generates_without_error() {
        let item: ItemImpl = parse_str(
            "impl Importer for MyPlugin {
                fn name(&self) -> &str { \"my-plugin\" }
                fn import(&self, _: ImportConfig)
                    -> Result<ImportOutput, ImportError> { Ok(ImportOutput::default()) }
            }",
        )
        .expect("test input is valid syn");
        assert!(
            generate_importer_export(&item, ExporterGlue::Unsupported).is_ok(),
            "trait impl should generate export glue successfully"
        );
    }

    #[test]
    fn non_trait_impl_returns_error() {
        let item: ItemImpl = parse_str("impl MyPlugin { fn name(&self) -> &str { \"x\" } }")
            .expect("test input is valid syn");
        assert!(
            generate_importer_export(&item, ExporterGlue::Unsupported).is_err(),
            "bare impl (no trait) should return a syn error"
        );
    }

    /// Every WIT export the world requires must appear in the generated glue.
    ///
    /// That the exports *delegate* to the trait rather than fabricating a
    /// result is asserted in `bc-sdk`'s `importer_macro` integration test,
    /// which compiles the macro and calls through it.
    #[test]
    fn generated_glue_exports_the_whole_world() {
        let item: ItemImpl = parse_str(
            "impl Importer for MyPlugin {
                fn name(&self) -> &str { \"my-plugin\" }
                fn import(&self, _: ImportConfig)
                    -> Result<ImportOutput, ImportError> { Ok(ImportOutput::default()) }
                fn validate(&self, _: ImportConfig) -> Result<(), ImportError> { Ok(()) }
            }",
        )
        .expect("test input is valid syn");
        let generated = generate_importer_export(&item, ExporterGlue::Unsupported)
            .expect("trait impl should generate")
            .to_string();
        for export in ["fn sdk_abi", "fn name", "fn parse", "fn validate"] {
            assert!(
                generated.contains(export),
                "generated Guest impl is missing {export}, so the component \
                 will not satisfy the world"
            );
        }
    }

    /// A plugin that does not export still satisfies the world, by reporting
    /// that it cannot.
    #[test]
    fn default_glue_exports_an_unsupported_exporter() {
        let generated = generate_importer_export(&sample_impl(), ExporterGlue::Unsupported)
            .expect("trait impl should generate")
            .to_string();
        assert!(
            generated.contains("ExportError :: Unsupported"),
            "default glue must answer every export with Unsupported"
        );
        assert!(
            !generated.contains("as :: bc_sdk :: Exporter"),
            "default glue must not require an Exporter impl"
        );
    }

    #[test]
    fn exporter_flag_delegates_to_the_trait() {
        let mode = parse_attr(quote! { exporter }).expect("`exporter` is a valid argument");
        let generated = generate_importer_export(&sample_impl(), mode)
            .expect("trait impl should generate")
            .to_string();
        assert!(
            generated.contains("as :: bc_sdk :: Exporter"),
            "exporter glue must call the Exporter trait"
        );
        assert!(
            !generated.contains("ExportError :: Unsupported"),
            "exporter glue must not fabricate an Unsupported answer"
        );
    }

    #[test]
    fn empty_attribute_selects_the_default_glue() {
        let mode = parse_attr(TokenStream2::new()).expect("no argument is valid");
        assert!(
            mode == ExporterGlue::Unsupported,
            "no argument selects the default glue"
        );
    }

    #[test]
    fn unknown_attribute_is_an_error() {
        assert!(
            parse_attr(quote! { bogus }).is_err(),
            "an unrecognised argument should be a syn error"
        );
    }

    #[test]
    fn extra_attribute_tokens_are_an_error() {
        assert!(
            parse_attr(quote! { exporter, exporter }).is_err(),
            "a second argument should be a syn error"
        );
    }
}

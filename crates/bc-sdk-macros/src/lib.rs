//! Proc-macro support for `bc-sdk`.
//!
//! Provides the `#[importer]` attribute macro that generates WIT export glue
//! for types implementing [`bc_sdk::Importer`].

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::ItemImpl;
use syn::parse_macro_input;

/// Generates WASM Component Model export glue for an `impl Importer` block.
///
/// Apply this attribute to an `impl bc_sdk::Importer for YourType` block.
/// The implementing type must also implement [`Default`].
///
/// # Example
///
/// ```rust,ignore
/// use bc_sdk::{ImportConfig, ImportError, Importer, RawTransaction};
///
/// #[derive(Default)]
/// struct CsvImporter;
///
/// #[bc_sdk::importer]
/// impl Importer for CsvImporter {
///     fn name(&self) -> &str { "csv" }
///     fn import(&self, config: ImportConfig) -> Result<Vec<RawTransaction>, ImportError> {
///         Ok(vec![])
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn importer(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let item_impl = parse_macro_input!(item as ItemImpl);
    match generate_importer_export(&item_impl) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// Generates the token stream for WIT export glue from an `impl Importer` block.
///
/// # Errors
///
/// Returns a [`syn::Error`] if the input is not a trait impl block.
fn generate_importer_export(item_impl: &ItemImpl) -> syn::Result<TokenStream2> {
    let self_ty = &item_impl.self_ty;

    if item_impl.trait_.is_none() {
        return Err(syn::Error::new_spanned(
            item_impl,
            "#[importer] must be applied to `impl Importer for YourType`",
        ));
    }

    let export_struct = quote! { __BcImporterExport };

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
                ::std::vec::Vec<
                    ::bc_sdk::__bindings::exports::borrow_checker::sdk::importer::RawTransaction
                >,
                ::bc_sdk::__bindings::exports::borrow_checker::sdk::importer::ImportError,
            > {
                let config = ::bc_sdk::ImportConfig::from_json_string(config);
                <#self_ty as ::bc_sdk::Importer>::import(
                    &<#self_ty as ::std::default::Default>::default(),
                    config,
                )
                .map(|txs| txs.into_iter().map(::core::convert::Into::into).collect())
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

        ::bc_sdk::export!(#export_struct with_types_in ::bc_sdk::__bindings);
    };

    Ok(expanded)
}

#[cfg(test)]
mod tests {
    use syn::parse_str;

    use super::*;

    #[test]
    fn trait_impl_generates_without_error() {
        let item: ItemImpl = parse_str(
            "impl Importer for MyPlugin {
                fn name(&self) -> &str { \"my-plugin\" }
                fn import(&self, _: ImportConfig)
                    -> Result<Vec<RawTransaction>, ImportError> { Ok(vec![]) }
            }",
        )
        .expect("test input is valid syn");
        assert!(
            generate_importer_export(&item).is_ok(),
            "trait impl should generate export glue successfully"
        );
    }

    #[test]
    fn non_trait_impl_returns_error() {
        let item: ItemImpl = parse_str("impl MyPlugin { fn name(&self) -> &str { \"x\" } }")
            .expect("test input is valid syn");
        assert!(
            generate_importer_export(&item).is_err(),
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
                    -> Result<Vec<RawTransaction>, ImportError> { Ok(vec![]) }
                fn validate(&self, _: ImportConfig) -> Result<(), ImportError> { Ok(()) }
            }",
        )
        .expect("test input is valid syn");
        let generated = generate_importer_export(&item)
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
}

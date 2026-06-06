//! Proc-macros for SystemRS.
//!
//! This is the L0 proc-macro crate (`doc/systemrs-design.md` §10.1). It has **no**
//! `systemrs-*` dependencies and emits fully `::systemrs::`-path-qualified code, so
//! the umbrella `systemrs` facade can re-export these macros without a dependency
//! cycle (the macro crate never names the facade as a dependency; it only writes the
//! path as tokens, resolved in the *user's* crate where `systemrs` is in scope).

use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, parse_macro_input};

/// Marks a struct as a SystemRS module, generating its `Module` marker impl.
///
/// `#[module]` is optional sugar over a hand-written `impl systemrs::Module`: it
/// emits the struct unchanged plus `impl ::systemrs::Module for T {}`. Because
/// `Module: Elaborate`, the type must also implement
/// [`systemrs::Elaborate`](../systemrs/trait.Elaborate.html) (write the lifecycle
/// callbacks there, or `impl Elaborate for T {}` for a module with no callbacks).
///
/// ```ignore
/// use systemrs::module;
/// use systemrs::prelude::*;
///
/// #[module]
/// struct Cpu {
///     // sub-components …
/// }
///
/// impl Elaborate for Cpu {
///     fn end_of_elaboration(&mut self, ctx: &Ctx) { /* … */ }
/// }
/// ```
///
/// # Arguments
///
/// * `_attr` - Unused attribute arguments.
/// * `item` - The annotated `struct` definition.
///
/// # Returns
///
/// The original struct followed by its generated `Module` impl.
#[proc_macro_attribute]
pub fn module(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as DeriveInput);
    let ident = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let expanded = quote! {
        #input

        impl #impl_generics ::systemrs::Module for #ident #ty_generics #where_clause {}
    };
    expanded.into()
}

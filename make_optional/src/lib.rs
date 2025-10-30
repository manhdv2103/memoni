use std::mem;

use proc_macro::TokenStream;
use proc_macro2::{Literal, TokenTree};
use quote::quote;
use syn::{
    Attribute, Data, DeriveInput, Fields, Ident, LitStr, Meta, Token, Type, Visibility,
    parenthesized, parse::Parse, parse_macro_input, parse_str,
};

#[proc_macro_derive(MakeOptional, attributes(optional))]
pub fn make_optional(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let attrs = &input.attrs;
    let fields = match &input.data {
        Data::Struct(s) => &s.fields,
        _ => {
            return syn::Error::new_spanned(name, "`MakeOptional` only works on structs")
                .to_compile_error()
                .into();
        }
    };

    let mut optional_vis = input.vis;
    let (optional_attrs, attrs): (Vec<_>, Vec<_>) = attrs
        .clone()
        .into_iter()
        .partition(|attr| attr.path().is_ident("optional"));

    let mut extra_derive_idents = vec![];
    for attr in optional_attrs {
        match process_struct_optional_attr(attr) {
            Ok((mut derives, vis)) => {
                extra_derive_idents.append(&mut derives);
                if let Some(vis) = vis {
                    optional_vis = vis;
                }
            }
            Err(err) => return err.to_compile_error().into(),
        }
    }

    let Fields::Named(named_fields) = fields else {
        return syn::Error::new_spanned(fields, "`MakeOptional` only supports named fields")
            .to_compile_error()
            .into();
    };

    let mut optional_fields = vec![];
    let mut field_applies = vec![];
    for field in &named_fields.named {
        let ident = &field.ident;
        let (attrs, use_optional_type) = match process_field_attrs(&field.attrs) {
            Ok(res) => res,
            Err(err) => return err.to_compile_error().into(),
        };

        let mut ty = field.ty.clone();
        if use_optional_type {
            let Type::Path(ref mut path) = ty else {
                return syn::Error::new_spanned(ty, "unsupported type for `optional_type` option")
                    .to_compile_error()
                    .into();
            };

            if let Some(last) = path.path.segments.last_mut() {
                let ident_str = last.ident.to_string();
                last.ident = Ident::new(&format!("Optional{}", ident_str), last.ident.span())
            }
        }

        optional_fields.push(quote! {
            #(#attrs)*
            #ident: Option<#ty>
        });

        field_applies.push(if use_optional_type {
            quote! {
                if let Some(v) = optional.#ident {
                    self.#ident.apply_optional(v);
                }
            }
        } else {
            quote! {
                if let Some(v) = optional.#ident {
                    self.#ident = v;
                }
            }
        });
    }

    let optional_name = Ident::new(&format!("Optional{name}"), name.span());
    quote! {
        #[derive(#(#extra_derive_idents),*)]
        #(#attrs)*
        #optional_vis struct #optional_name {
            #(#optional_fields,)*
        }

        impl #name {
            pub fn apply_optional(&mut self, optional: #optional_name) {
                #(#field_applies)*
            }

            pub fn with_optional(mut self, optional: #optional_name) -> Self {
                self.apply_optional(optional);
                self
            }
        }
    }
    .into()
}

fn process_struct_optional_attr(attr: Attribute) -> syn::Result<(Vec<Ident>, Option<Visibility>)> {
    let mut derive_idents = vec![];
    let mut vis = None;
    attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("derive") {
            let content;
            parenthesized!(content in meta.input);
            let idents = content.parse_terminated(Ident::parse, Token![,])?;
            derive_idents.extend(idents);
            return Ok(());
        }

        if meta.path.is_ident("vis") {
            let content;
            parenthesized!(content in meta.input);
            vis = Some(content.parse()?);
            return Ok(());
        }

        Err(meta.error("unrecognized attribute `optional` option"))
    })?;

    Ok((derive_idents, vis))
}

fn process_field_attrs(attrs: &Vec<Attribute>) -> syn::Result<(Vec<Attribute>, bool)> {
    let mut processed_attrs = vec![];
    let mut use_optional_type = false;

    for attr in attrs {
        let attr = attr.clone();
        let attr = if attr.path().is_ident("optional") {
            use_optional_type = process_field_optional_attr(attr)?;
            None
        } else if attr.path().is_ident("serde") {
            process_serde_attr(attr)
        } else if attr.path().is_ident("serde_as") {
            process_serde_as_attr(attr)
        } else {
            Some(attr)
        };

        if let Some(attr) = attr {
            processed_attrs.push(attr)
        }
    }

    Ok((processed_attrs, use_optional_type))
}

fn process_field_optional_attr(attr: Attribute) -> syn::Result<bool> {
    let mut use_optional_type = false;
    attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("optional_type") {
            use_optional_type = true;
            return Ok(());
        }

        Err(meta.error("unrecognized attribute `optional` option"))
    })?;

    Ok(use_optional_type)
}

fn process_serde_attr(mut attr: Attribute) -> Option<Attribute> {
    if let Meta::List(meta) = &mut attr.meta {
        let mut new_meta_tokens = vec![];
        let mut default_tokens = false;
        for token in mem::take(&mut meta.tokens) {
            if let TokenTree::Ident(ref ident) = token {
                default_tokens = ident == "default";
            }

            if !default_tokens {
                new_meta_tokens.push(token);
            }
        }

        if new_meta_tokens.is_empty() {
            return None;
        }

        meta.tokens.extend(new_meta_tokens);
    }

    Some(attr)
}

fn process_serde_as_attr(mut attr: Attribute) -> Option<Attribute> {
    if let Meta::List(meta) = &mut attr.meta {
        let mut new_meta_tokens = vec![];
        let mut as_tokens = false;
        for token in mem::take(&mut meta.tokens) {
            if let TokenTree::Ident(ref ident) = token {
                as_tokens = ident == "as";
            }

            if as_tokens
                && let TokenTree::Literal(ref literal) = token
                && let Ok(str_literal) = parse_str::<LitStr>(&literal.to_string())
            {
                new_meta_tokens.push(TokenTree::Literal(Literal::string(&format!(
                    "Option<{}>",
                    str_literal.value()
                ))));
            } else {
                new_meta_tokens.push(token);
            }
        }

        meta.tokens.extend(new_meta_tokens);
    }

    Some(attr)
}

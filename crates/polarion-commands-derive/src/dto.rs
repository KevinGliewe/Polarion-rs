//! `#[cmd]` and `#[cmd_args]`.

use heck::ToKebabCase;
use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::Parser as _;
use syn::punctuated::Punctuated;
use syn::{
    Attribute, Expr, ExprLit, Field, Fields, FieldsNamed, GenericArgument, ItemStruct, Lit, Meta,
    PathArguments, Token, Type,
};

/// What the line-oriented rules need to know about a field's type.
enum Shape {
    Bool,
    OptionBool,
    Option,
    Vec,
    /// `TextArg`: the text itself, or on the command line `@FILE`.
    Text,
    Other,
}

fn shape(ty: &Type) -> Shape {
    let Type::Path(path) = ty else {
        return Shape::Other;
    };
    let Some(last) = path.path.segments.last() else {
        return Shape::Other;
    };
    match last.ident.to_string().as_str() {
        "bool" => Shape::Bool,
        "Vec" => Shape::Vec,
        "TextArg" => Shape::Text,
        "Option" => {
            let inner_bool = matches!(&last.arguments, PathArguments::AngleBracketed(args)
                if matches!(args.args.first(), Some(GenericArgument::Type(Type::Path(p)))
                    if p.path.is_ident("bool")));
            if inner_bool {
                Shape::OptionBool
            } else {
                Shape::Option
            }
        }
        _ => Shape::Other,
    }
}

/// The command's options from `#[cmd(...)]`.
#[derive(Default)]
struct Options {
    write: bool,
    view: Option<TokenStream>,
}

fn options(attr: TokenStream) -> syn::Result<Options> {
    let mut options = Options::default();
    let metas = Punctuated::<Meta, Token![,]>::parse_terminated.parse2(attr)?;
    for meta in metas {
        match &meta {
            Meta::Path(p) if p.is_ident("write") => options.write = true,
            Meta::NameValue(nv) if nv.path.is_ident("view") => {
                options.view = Some(view(&nv.value)?);
            }
            other => {
                return Err(syn::Error::new_spanned(
                    other,
                    "expected `write` or `view = ...`",
                ));
            }
        }
    }
    Ok(options)
}

/// `table(COLUMNS)`, `object`, ... as a `crate::View` expression.
fn view(expr: &Expr) -> syn::Result<TokenStream> {
    let (name, arg) = match expr {
        Expr::Path(p) => (p.path.get_ident().cloned(), None),
        Expr::Call(call) => match &*call.func {
            Expr::Path(p) if call.args.len() == 1 => {
                (p.path.get_ident().cloned(), call.args.first())
            }
            _ => (None, None),
        },
        _ => (None, None),
    };
    let variant = match name.as_ref().map(ToString::to_string).as_deref() {
        Some("object") => quote!(Object),
        Some("lines") => quote!(Lines),
        Some("links") => quote!(Links),
        Some("scalar") => quote!(Scalar),
        Some("text") => quote!(Text),
        Some("bytes") => quote!(Bytes),
        Some("done") => quote!(Done),
        Some("table") if arg.is_some() => return Ok(quote!(crate::View::Table(#arg))),
        _ => {
            return Err(syn::Error::new_spanned(
                expr,
                "expected object, lines, links, scalar, text, bytes, done or table(COLUMNS)",
            ));
        }
    };
    if arg.is_some() {
        return Err(syn::Error::new_spanned(expr, "only `table` takes columns"));
    }
    Ok(quote!(crate::View::#variant))
}

/// Added to the description of commands that modify data.
const WRITE_NOTE: &str = "(Modifies data: needs POLARION_WRITE_PROJECTS for the project, or \
                          POLARION_WRITE_TARGETS naming the object.)";

/// What the command line adds to the help of a `TextArg` option (JSON never reads files, so the
/// doc comment, which is also the JSON Schema description, does not say it).
const TEXT_NOTE: &str =
    "(also @FILE for a file's content, @- for standard input, @@... for text that starts with @)";

/// The lines of the doc comment, trimmed.
fn doc_lines(attrs: &[Attribute]) -> Vec<String> {
    attrs
        .iter()
        .filter_map(|attr| match &attr.meta {
            Meta::NameValue(nv) if nv.path.is_ident("doc") => match &nv.value {
                Expr::Lit(ExprLit {
                    lit: Lit::Str(s), ..
                }) => Some(s.value().trim().to_owned()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// The first paragraph of the doc comment, on one line.
fn summary(attrs: &[Attribute]) -> String {
    let mut lines = Vec::new();
    for line in doc_lines(attrs) {
        if line.is_empty() {
            if lines.is_empty() {
                continue;
            }
            break;
        }
        lines.push(line);
    }
    lines.join(" ")
}

/// Whether the doc comment has more paragraphs than the summary (the command line's `--help`
/// shows all of them).
fn has_details(attrs: &[Attribute]) -> bool {
    let lines = doc_lines(attrs);
    let mut paragraphs = lines
        .split(|line| line.is_empty())
        .filter(|paragraph| !paragraph.is_empty());
    paragraphs.next().is_some() && paragraphs.next().is_some()
}

/// Add the serde and clap attributes a field needs, consuming `#[positional]` and `#[flatten]`.
fn field(field: &mut Field) -> syn::Result<()> {
    let mut positional = false;
    let mut flatten = false;
    field.attrs.retain(|attr| {
        if attr.path().is_ident("positional") {
            positional = true;
            false
        } else if attr.path().is_ident("flatten") {
            flatten = true;
            false
        } else {
            true
        }
    });
    let Some(ident) = &field.ident else {
        return Err(syn::Error::new_spanned(field, "fields must be named"));
    };
    let name = ident.to_string();
    let mut extra: Vec<Attribute> = Vec::new();
    let mut push = |tokens: TokenStream| -> syn::Result<()> {
        extra.extend(Attribute::parse_outer.parse2(tokens)?);
        Ok(())
    };
    if flatten {
        push(quote! {
            #[serde(flatten)]
            #[cfg_attr(feature = "cli", command(flatten))]
        })?;
    } else {
        let cli = if name == "project" {
            // The global `-p`/`--project` sets the default project instead.
            quote!(#[cfg_attr(feature = "cli", arg(skip))])
        } else if positional {
            quote!()
        } else {
            match shape(&field.ty) {
                Shape::OptionBool => quote! {
                    #[cfg_attr(feature = "cli", arg(long, num_args = 0..=1, require_equals = true,
                        default_missing_value = "true"))]
                },
                Shape::Vec => {
                    quote!(#[cfg_attr(feature = "cli", arg(long, value_delimiter = ','))])
                }
                // Markdown often starts with `-` (a list, a rule): it is the value, not an option.
                Shape::Text => {
                    let doc = doc_lines(&field.attrs).join(" ");
                    let help = format!("{doc} {TEXT_NOTE}").trim_start().to_owned();
                    quote! {
                        #[cfg_attr(feature = "cli", arg(long, allow_hyphen_values = true,
                            help = #help))]
                    }
                }
                _ => quote!(#[cfg_attr(feature = "cli", arg(long))]),
            }
        };
        push(cli)?;
        match shape(&field.ty) {
            Shape::Bool => {
                push(quote!(#[serde(default, skip_serializing_if = "::core::ops::Not::not")]))?
            }
            Shape::Vec => {
                push(quote!(#[serde(default, skip_serializing_if = "::std::vec::Vec::is_empty")]))?
            }
            Shape::Option | Shape::OptionBool => {
                push(
                    quote!(#[serde(default, skip_serializing_if = "::core::option::Option::is_none")]),
                )?;
            }
            Shape::Text | Shape::Other => {}
        }
    }
    field.attrs.extend(extra);
    Ok(())
}

pub fn expand(attr: TokenStream, item: TokenStream, command: bool) -> syn::Result<TokenStream> {
    let options = if command {
        options(attr)?
    } else if attr.is_empty() {
        Options::default()
    } else {
        return Err(syn::Error::new_spanned(
            attr,
            "`#[cmd_args]` takes no options",
        ));
    };
    let mut item: ItemStruct = syn::parse2(item)?;
    if !item.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &item.generics,
            "commands cannot be generic",
        ));
    }
    if matches!(item.fields, Fields::Unit) {
        item.fields = Fields::Named(FieldsNamed {
            brace_token: Default::default(),
            named: Punctuated::new(),
        });
    }
    let Fields::Named(fields) = &mut item.fields else {
        return Err(syn::Error::new_spanned(
            &item.fields,
            "commands need named fields",
        ));
    };
    for f in &mut fields.named {
        field(f)?;
    }

    let derives = quote! {
        #[derive(
            ::core::fmt::Debug,
            ::core::clone::Clone,
            ::serde::Serialize,
            ::serde::Deserialize,
            ::schemars::JsonSchema
        )]
        #[cfg_attr(feature = "cli", derive(::clap::Args))]
        #[serde(rename_all = "kebab-case")]
    };
    let info = if command {
        let ident = &item.ident;
        let name = ident.to_string().to_kebab_case();
        let summary = summary(&item.attrs);
        if summary.is_empty() {
            return Err(syn::Error::new_spanned(
                ident,
                "commands need a doc comment (its first paragraph is the summary)",
            ));
        }
        let description = if options.write {
            format!("{summary} {WRITE_NOTE}")
        } else {
            summary.clone()
        };
        if options.write && has_details(&item.attrs) {
            // `--help` shows the whole doc comment instead of the description: the note ends it.
            let note = format!(" {WRITE_NOTE}");
            item.attrs
                .extend(Attribute::parse_outer.parse2(quote!(#[doc = ""] #[doc = #note]))?);
        }
        let write = options.write;
        let view = options.view.unwrap_or_else(|| quote!(crate::View::Object));
        quote! {
            impl crate::CommandInfo for #ident {
                const NAME: &'static str = #name;
                const SUMMARY: &'static str = #summary;
                const DESCRIPTION: &'static str = #description;
                const WRITE: bool = #write;
                const VIEW: crate::View = #view;
            }
        }
    } else {
        quote!()
    };
    Ok(quote! {
        #derives
        #item
        #info
    })
}

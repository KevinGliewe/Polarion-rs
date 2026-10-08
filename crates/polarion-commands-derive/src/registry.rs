//! `registry!`.

use heck::{ToKebabCase, ToUpperCamelCase};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Attribute, Expr, ExprLit, Ident, Lit, Meta, Path, Token, bracketed};

/// `/// doc` `area [= module] => [Type, ...]`.
struct Area {
    doc: String,
    name: Ident,
    module: Path,
    commands: Vec<Ident>,
}

struct Registry {
    areas: Vec<Area>,
}

fn doc(attrs: &[Attribute]) -> String {
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
        .collect::<Vec<_>>()
        .join(" ")
}

impl Parse for Area {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let name: Ident = input.parse()?;
        let module = if input.peek(Token![=]) && !input.peek(Token![=>]) {
            input.parse::<Token![=]>()?;
            input.parse()?
        } else {
            Path::from(name.clone())
        };
        input.parse::<Token![=>]>()?;
        let content;
        bracketed!(content in input);
        let commands = Punctuated::<Ident, Token![,]>::parse_terminated(&content)?
            .into_iter()
            .collect();
        let doc = doc(&attrs);
        if doc.is_empty() {
            return Err(syn::Error::new_spanned(&name, "areas need a doc comment"));
        }
        Ok(Self {
            doc,
            name,
            module,
            commands,
        })
    }
}

impl Parse for Registry {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let areas = Punctuated::<Area, Token![,]>::parse_terminated(input)?
            .into_iter()
            .collect();
        Ok(Self { areas })
    }
}

/// One command: its variant in the generated enums, its type and its full name.
struct Entry<'a> {
    area: &'a Area,
    ty: &'a Ident,
    variant: Ident,
    full_name: String,
}

pub fn expand(input: TokenStream) -> syn::Result<TokenStream> {
    let registry: Registry = syn::parse2(input)?;
    let entries: Vec<Entry<'_>> = registry
        .areas
        .iter()
        .flat_map(|area| {
            area.commands.iter().map(move |ty| Entry {
                area,
                ty,
                variant: format_ident!("{}{}", area.name.to_string().to_upper_camel_case(), ty),
                full_name: format!("{}.{}", area.name, ty.to_string().to_kebab_case()),
            })
        })
        .collect();
    let mut seen = std::collections::HashSet::new();
    for e in &entries {
        if !seen.insert(&e.full_name) {
            return Err(syn::Error::new_spanned(
                e.ty,
                format!("`{}` is listed twice", e.full_name),
            ));
        }
    }

    let variants = entries.iter().map(|e| {
        let (variant, module, ty, full_name) = (&e.variant, &e.area.module, e.ty, &e.full_name);
        quote! {
            #[serde(rename = #full_name)]
            #variant(#module::#ty)
        }
    });
    let outputs = entries.iter().map(|e| {
        let (variant, module, ty) = (&e.variant, &e.area.module, e.ty);
        quote!(#variant(<#module::#ty as crate::Run>::Output))
    });
    let names = entries.iter().map(|e| {
        let (variant, full_name) = (&e.variant, &e.full_name);
        quote!(Self::#variant(_) => #full_name)
    });
    let output_names = entries.iter().map(|e| {
        let (variant, full_name) = (&e.variant, &e.full_name);
        quote!(Self::#variant(_) => #full_name)
    });
    let runs = entries.iter().map(|e| {
        let variant = &e.variant;
        quote!(Self::#variant(c) => crate::Run::run(c, cx).await.map(AnyOutput::#variant))
    });
    let catalog = entries.iter().map(|e| {
        let (variant, module, ty) = (&e.variant, &e.area.module, e.ty);
        let area = e.area.name.to_string();
        quote! {
            crate::Entry {
                area: #area,
                name: <#module::#ty as crate::CommandInfo>::NAME,
                summary: <#module::#ty as crate::CommandInfo>::SUMMARY,
                description: <#module::#ty as crate::CommandInfo>::DESCRIPTION,
                write: <#module::#ty as crate::CommandInfo>::WRITE,
                view: <#module::#ty as crate::CommandInfo>::VIEW,
                schema: crate::schema_of::<#module::#ty>,
                parse: |value| crate::parse_as::<#module::#ty>(value).map(AnyCommand::#variant),
            }
        }
    });
    let areas = registry.areas.iter().map(|a| {
        let (name, doc) = (a.name.to_string(), &a.doc);
        quote!((#name, #doc))
    });

    // The command line: `Cli` has one variant per area, each with an enum of its commands.
    let cli_areas = registry.areas.iter().map(|a| {
        let area_enum = format_ident!("{}Cli", a.name.to_string().to_upper_camel_case());
        let module = &a.module;
        let variants = a.commands.iter().map(|ty| {
            quote! {
                #[command(about = <#module::#ty as crate::CommandInfo>::DESCRIPTION)]
                #ty(#module::#ty)
            }
        });
        let doc = format!("The commands of `{}`.", a.name);
        quote! {
            #[doc = #doc]
            #[cfg(feature = "cli")]
            #[derive(::core::fmt::Debug, ::clap::Subcommand)]
            pub enum #area_enum {
                #(#variants),*
            }
        }
    });
    let cli_variants = registry.areas.iter().map(|a| {
        let variant = format_ident!("{}", a.name.to_string().to_upper_camel_case());
        let area_enum = format_ident!("{}Cli", a.name.to_string().to_upper_camel_case());
        let doc = &a.doc;
        quote! {
            #[command(about = #doc)]
            #variant {
                #[command(subcommand)]
                command: #area_enum,
            }
        }
    });
    let cli_conversions = registry.areas.iter().map(|a| {
        let variant = format_ident!("{}", a.name.to_string().to_upper_camel_case());
        let area_enum = format_ident!("{}Cli", a.name.to_string().to_upper_camel_case());
        let arms = a.commands.iter().map(|ty| {
            let any = format_ident!("{}{}", a.name.to_string().to_upper_camel_case(), ty);
            quote!(#area_enum::#ty(c) => AnyCommand::#any(c))
        });
        quote!(Cli::#variant { command } => match command { #(#arms),* })
    });
    let count = entries.len();

    Ok(quote! {
        /// Any command with its arguments. Serializes (in any serde format) as
        /// `{"command": "area.name", "args": {...}}`.
        #[derive(::core::fmt::Debug, ::core::clone::Clone, ::serde::Serialize, ::serde::Deserialize)]
        #[serde(tag = "command", content = "args")]
        pub enum AnyCommand {
            #(#variants),*
        }

        /// The result of any command, serialized as the result itself.
        #[derive(::serde::Serialize)]
        #[serde(untagged)]
        pub enum AnyOutput {
            #(#outputs),*
        }

        impl ::core::fmt::Debug for AnyOutput {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                let name = match self { #(#output_names),* };
                write!(f, "AnyOutput({name})")
            }
        }

        impl AnyCommand {
            /// The full name, `area.name`.
            pub fn name(&self) -> &'static str {
                match self { #(#names),* }
            }

            /// Run the command.
            pub async fn run(self, cx: &crate::Context<'_>) -> crate::Result<AnyOutput> {
                match self { #(#runs),* }
            }
        }

        /// Every command, grouped by area.
        pub static CATALOG: [crate::Entry; #count] = [#(#catalog),*];

        /// The areas, in catalog order, with a one-line description.
        pub const AREAS: &[(&str, &str)] = &[#(#areas),*];

        #(#cli_areas)*

        /// The command line: `polarion <area> <command> [args]`.
        #[cfg(feature = "cli")]
        #[derive(::core::fmt::Debug, ::clap::Subcommand)]
        pub enum Cli {
            #(#cli_variants),*
        }

        #[cfg(feature = "cli")]
        impl ::core::convert::From<Cli> for AnyCommand {
            fn from(cli: Cli) -> Self {
                match cli { #(#cli_conversions),* }
            }
        }
    })
}

//! Code generation for `polarion-commands`.
//!
//! - [`macro@cmd`] turns a struct into a command DTO: serde, JSON Schema and clap derives,
//!   per-field command-line and serialization attributes, and the command's metadata
//!   (`CommandInfo`: name from the type name, summary from the doc comment, write flag, view).
//! - [`macro@cmd_args`] does the same for a group of arguments shared by several commands
//!   (flattened into them with `#[flatten]`), without metadata.
//! - [`registry!`] lists the commands by area and generates the typed `AnyCommand` and
//!   `AnyOutput` enums, the catalog used by the front ends and the clap command tree.
//!
//! The generated code refers to `crate::...` items and is meant for `polarion-commands` only.

mod dto;
mod registry;

use proc_macro::TokenStream;

/// A command DTO: `#[cmd(write, view = table(ITEM_COLUMNS))]` on a struct with named fields.
///
/// Options: `write` (the command modifies data), `view = object | lines | links | scalar | text |
/// bytes | done | table(COLUMNS)` (how the command line shows the result; default `object`).
///
/// Fields: `#[positional]` takes the field positionally on the command line (in declaration
/// order); `#[flatten]` inlines a `#[cmd_args]` group. Every other field is a `--long` option;
/// a field named `project` is not an option at all (the global `-p` sets the default project).
/// `bool` fields are flags, `Option<bool>` fields take `--flag[=true|false]`, `Vec` fields
/// take comma-separated values.
#[proc_macro_attribute]
pub fn cmd(attr: TokenStream, item: TokenStream) -> TokenStream {
    dto::expand(attr.into(), item.into(), true)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// A group of arguments shared by several commands; see [`macro@cmd`] for the field rules.
#[proc_macro_attribute]
pub fn cmd_args(attr: TokenStream, item: TokenStream) -> TokenStream {
    dto::expand(attr.into(), item.into(), false)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// The command registry: `/// area description` then `area => [Type, ...]` (the module is the
/// area name) or `area = module => [Type, ...]`, separated by commas.
#[proc_macro]
pub fn registry(input: TokenStream) -> TokenStream {
    registry::expand(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

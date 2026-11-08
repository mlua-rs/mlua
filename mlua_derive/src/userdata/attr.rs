use proc_macro2::Span;
use syn::ext::IdentExt;
use syn::meta::ParseNestedMeta;
use syn::{Ident, LitStr, Result};

/// Parsed `#[lua(...)]` attribute.
///
/// Some flags are context-dependent:
/// - Struct fields: `get`, `set`, `name`, `skip`
/// - Impl methods: `getter`, `setter`, `field`, `meta`, `infallible`, `name`, `skip`
#[derive(Default)]
pub(crate) struct LuaAttr {
    pub(crate) span: Option<Span>,
    pub(crate) name: Option<String>,
    pub(crate) infallible: bool,
    pub(crate) skip: bool,

    // Struct field context flags
    pub(crate) get: bool,
    pub(crate) set: bool,

    // Impl method context flags
    pub(crate) getter: bool,
    pub(crate) setter: bool,
    pub(crate) field: bool,
    pub(crate) meta: bool,
}

impl LuaAttr {
    pub(crate) fn parse_inner(&mut self, meta: ParseNestedMeta) -> Result<()> {
        match &meta.path {
            path if path.is_ident("skip") => {
                if meta.value().is_ok() {
                    return Err(meta.error("`skip` does not take a value"));
                }
                self.skip = true;
            }
            path if path.is_ident("infallible") => {
                if meta.value().is_ok() {
                    return Err(meta.error("`infallible` does not take a value"));
                }
                self.infallible = true;
            }
            path if path.is_ident("get") => self.get = true,
            path if path.is_ident("set") => self.set = true,
            path if path.is_ident("getter") => self.getter = true,
            path if path.is_ident("setter") => self.setter = true,
            path if path.is_ident("field") => self.field = true,
            path if path.is_ident("meta") => self.meta = true,
            path if path.is_ident("name") => {
                let value = meta.value()?;
                let lit: LitStr = value.parse()?;
                self.name = Some(lit.value());
            }
            _ => {
                return Err(meta.error(
                    "unsupported lua attribute, expected: ".to_string()
                        + "`skip`, `infallible`, `get`, `set`, `getter`, `setter`, `field`, `meta`, `name`",
                ));
            }
        }
        Ok(())
    }

    /// Returns the effective Lua name.
    pub(crate) fn name(&self, ident: &Ident) -> String {
        self.name.clone().unwrap_or_else(|| ident.unraw().to_string())
    }

    /// Returns the span to use for error reporting.
    pub(crate) fn span(&self) -> Span {
        self.span.unwrap_or_else(Span::call_site)
    }

    /// Returns the effective Lua metamethod name.
    ///
    /// If `name` is set via attribute, use it. Otherwise, if the function name
    /// starts with `__`, use that. Returns an error if neither is available.
    pub(crate) fn effective_meta_name(&self, fn_ident: &Ident) -> Result<String> {
        if let Some(ref name) = self.name {
            return Ok(name.clone());
        }
        let fn_name = fn_ident.unraw().to_string();
        if fn_name.starts_with("__") {
            return Ok(fn_name);
        }
        Err(syn::Error::new(
            fn_ident.span(),
            format!(
                "could not infer metamethod name from `{fn_name}`, either add `name = \"...\"` to `#[lua(meta, ...)]` or prefix the function with `__`"
            ),
        ))
    }
}

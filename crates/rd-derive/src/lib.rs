use darling::{
    ast::{self, Data},
    FromDeriveInput, FromField, FromVariant,
};
use proc_macro2::{Literal, Span, TokenStream};
use quote::{quote, ToTokens};
use syn::{parse_macro_input, DeriveInput, Ident};

#[derive(Debug, FromField)]
struct MyFieldReceiver {
    ident: Option<syn::Ident>,
}

#[derive(Debug, FromVariant)]
struct MyVariantReceiver {
    ident: syn::Ident,

    fields: ast::Fields<MyFieldReceiver>,
}

#[derive(Debug, FromDeriveInput)]
struct RDConfigReceiver {
    ident: syn::Ident,
    generics: syn::Generics,
    data: ast::Data<MyVariantReceiver, MyFieldReceiver>,
}

impl MyFieldReceiver {
    fn to_token(&self, method_path: &TokenStream, args: &TokenStream) -> TokenStream {
        let field_name = &self.ident.clone().unwrap();
        let name = Literal::string(&field_name.to_string());
        quote! {
            ctx.push(#name);
            #method_path(&mut self.#field_name, #args)?;
            ctx.pop();
        }
    }
}

impl RDConfigReceiver {
    fn call_all(&self, method_path: TokenStream, args: TokenStream) -> TokenStream {
        let ident = &self.ident;
        let mut body = quote! {};

        match &self.data {
            Data::Struct(s) => {
                let fields = &s.fields;

                for field in fields {
                    body.extend(field.to_token(&method_path, &args));
                }
            }
            Data::Enum(variants) => {
                for variant in variants {
                    let variant_name = &variant.ident;
                    let mut inner = TokenStream::new();
                    let mut head = TokenStream::new();

                    match &variant.fields.style {
                        ast::Style::Struct => {
                            for field in &variant.fields.fields {
                                let field_name = &field.ident.clone().unwrap();
                                let name = Literal::string(&field_name.to_string());
                                head.extend(quote! { #field_name, });
                                inner.extend(quote! {
                                    ctx.push(#name);
                                    #method_path(#field_name, #args)?;
                                    ctx.pop();
                                });
                            }
                            head = quote! { { #head } };
                        }
                        ast::Style::Tuple => {
                            for (i, _f) in variant.fields.fields.iter().enumerate() {
                                let field_name = Ident::new(&format!("i{}", i), Span::call_site())
                                    .to_token_stream();
                                let name = Literal::string(&field_name.to_string());
                                head.extend(quote! { #field_name, });
                                inner.extend(quote! {
                                    ctx.push(#name);
                                    #method_path(#field_name, #args)?;
                                    ctx.pop();
                                });
                            }
                            head = quote! { (#head) };
                        }
                        ast::Style::Unit => {}
                    }

                    body.extend(quote! {
                        #ident::#variant_name #head => {
                            #inner
                        }
                    })
                }

                body = quote! {
                    match self {
                        #body
                    }
                };
            }
        }

        body
    }
}

impl ToTokens for RDConfigReceiver {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let ident = &self.ident;
        let (impl_generics, ty_generics, where_clause) = self.generics.split_for_impl();

        let visitor_body = self.call_all(
            quote! { rd_interface::config::Config::visit },
            quote! { ctx, visitor },
        );

        let expanded = quote! {
            impl #impl_generics rd_interface::config::Config for #ident #ty_generics #where_clause {
                fn visit(&mut self, ctx: &mut rd_interface::config::VisitorContext, visitor: &mut dyn rd_interface::config::Visitor) -> rd_interface::Result<()> {
                    #visitor_body
                    Ok(())
                }
            }
        };

        tokens.extend(expanded)
    }
}

#[proc_macro_derive(Config)]
pub fn config(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let receiver = RDConfigReceiver::from_derive_input(&input).unwrap();

    quote!(#receiver).into()
}

#[proc_macro_attribute]
pub fn rd_config(
    _metadata: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let input: TokenStream = input.into();
    let output = quote! {
        #[derive(serde::Serialize, serde::Deserialize, rd_interface::Config, rd_interface::schemars::JsonSchema)]
        #input
    };
    output.into()
}

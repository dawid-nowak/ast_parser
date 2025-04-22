use std::env;
use std::fs;
use std::process;

use itertools::Itertools;
use multimap::MultiMap;
use proc_macro2::{Ident, Span};
use syn::File;
use syn::Item;
use syn::ItemStruct;
use syn::PathSegment;
use syn::Type;
use syn::visit::{self, Visit};

struct StructVisitor<'ast> {
    structs: Vec<&'ast ItemStruct>,
}

fn check_simple_type(path: &PathSegment, is_simple: &mut bool) {
    if path.arguments.is_empty() {
        let ident = &path.ident;
        if ident == &Ident::new("String", Span::call_site())
            || ident == &Ident::new("i32", Span::call_site())
        {
        } else {
            *is_simple = false;
        }
    } else {
        match &path.arguments {
            syn::PathArguments::None => *is_simple = false,
            syn::PathArguments::AngleBracketed(angle_bracketed_generic_arguments) => {
                for a in &angle_bracketed_generic_arguments.args {
                    match a {
                        syn::GenericArgument::Type(Type::Path(path)) => {
                            for s in &path.path.segments {
                                check_simple_type(s, is_simple);
                            }
                        }
                        _ => *is_simple = false,
                    }
                }
            }
            syn::PathArguments::Parenthesized(_) => *is_simple = false,
        }
    }
}

impl<'ast> Visit<'ast> for StructVisitor<'ast> {
    fn visit_item_struct(&mut self, node: &'ast ItemStruct) {
        println!("Visiting Struct name == {}", node.ident);
        let mut is_simple_leaf = true;
        node.fields.iter().for_each(|f| match &f.ty {
            Type::Path(path_type) => {
                println!(
                    "\twith field name = {:?} \n\t\tfield type = {:?}",
                    f.ident, f.ty
                );

                for segment in &path_type.path.segments {
                    check_simple_type(segment, &mut is_simple_leaf);
                }
            }

            _ => {
                is_simple_leaf = false;
            }
        });
        println!("Visiting Struct name == {} {is_simple_leaf}", node.ident);
        if is_simple_leaf {
            self.structs.push(node);
        }
        visit::visit_item_struct(self, node);
    }
}

pub fn longest_common_prefix(strs: &[String]) -> String {
    if strs.is_empty() {
        return String::new();
    }

    let mut prefix = strs[0].clone();

    while !prefix.is_empty() {
        if strs.iter().any(|s| !s.starts_with(&prefix)) {
            prefix.pop(); // Shorten the prefix
        } else {
            return prefix;
        }
    }

    prefix
}

fn main() {
    let mut args = env::args();
    let _ = args.next(); // executable name

    let (http_routes_filename, grpc_routes_filename) = match (args.next(), args.next(), args.next())
    {
        (Some(http_routes_filename), Some(grpc_routes_filename), None) => {
            (http_routes_filename, grpc_routes_filename)
        }
        _ => {
            eprintln!("Usage: dump-syntax path/to/filename.rs");
            process::exit(1);
        }
    };

    let http_routes_src = fs::read_to_string(&http_routes_filename).expect("unable to read file");
    let grpc_routes_src = fs::read_to_string(&grpc_routes_filename).expect("unable to read file");

    let http_routes_syntaxt = syn::parse_file(&http_routes_src).expect("unable to parse file");
    let grpc_routes_syntaxt = syn::parse_file(&grpc_routes_src).expect("unable to parse file");
    let mut http_visitor = StructVisitor {
        structs: Vec::new(),
    };

    let mut grpc_visitor = StructVisitor {
        structs: Vec::new(),
    };

    http_visitor.visit_file(&http_routes_syntaxt);
    grpc_visitor.visit_file(&grpc_routes_syntaxt);

    let mut potentially_similar_items = MultiMap::new();
    for i in http_visitor.structs.iter().chain(&grpc_visitor.structs) {
        potentially_similar_items.insert(&i.fields, (&i.ident, i));
    }

    let _items: Vec<_> = potentially_similar_items
        .iter_all()
        .filter_map(|(k, v)| {
            println!(
                "Similar items {:?}",
                v.iter().map(|v| v.0.to_string()).collect::<Vec<_>>()
            );
            let reversed_names: Vec<_> = v
                .iter()
                .map(|v| v.0.to_string().chars().rev().collect())
                .sorted_by(|r: &String, l: &String| Ord::cmp(&l.len(), &r.len()))
                .collect();
            let longest_prefix = longest_common_prefix(&reversed_names)
                .chars()
                .rev()
                .collect::<String>();
            println!("Longest prefix {longest_prefix}");

            if let Some((_i, s)) = v.first() {
                let mut new_struct = (**s).clone();
                new_struct.ident = Ident::new(
                    &format!("Shared{}", longest_prefix.chars().rev().collect::<String>()),
                    Span::call_site(),
                );
                Some(Item::Struct(new_struct))
            } else {
                None
            }
        })
        .collect();

    // let out = prettyplease::unparse(&File {
    //     shebang: None,
    //     attrs: vec![],
    //     items,
    // });

    // println!("\n\n\n Out code {out}");

    // Debug impl is available if Syn is built with "extra-traits" feature.
    //println!("{:#?}", syntax);
}

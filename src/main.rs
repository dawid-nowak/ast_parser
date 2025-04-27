use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::process;

use itertools::Itertools;
use itertools::PeekingNext;
use multimap::MultiMap;
use proc_macro2::{Ident, Span};
use syn::AttrStyle;
use syn::Fields;
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
        println!("Visiting Struct name == {:#?}", node);
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

fn break_into_words(type_name: &str) -> Vec<String> {
    let mut words = vec![];
    let mut current_word = String::new();

    for t in type_name.chars().tuple_windows() {
        let (current, next, next_next) = t;
        if current.is_uppercase() {
            if next.is_uppercase() {
                current_word.push(current);
                if !next_next.is_uppercase() {
                    words.push(current_word);
                    current_word = String::new();
                }
            } else {
                current_word.push(current);
            }
        } else {
            current_word.push(current);
            if next.is_uppercase() {
                words.push(current_word);
                current_word = String::new();
            }
        }
    }
    let len = type_name.len() - 2;
    if len > 0 {
        current_word += &type_name[len..];
        words.push(current_word);
    } else {
        words.push(type_name.to_owned());
    }

    words
}

pub fn common_words(words_sets: &[Vec<String>]) -> Vec<String> {
    println!("Words {words_sets:?}");
    let word_sets: Vec<BTreeSet<String>> = words_sets
        .iter()
        .cloned()
        .map(BTreeSet::from_iter)
        .collect();

    let mut intersection = if let Some(first) = word_sets.first() {
        first.clone()
    } else {
        return vec![];
    };

    for word_set in word_sets {
        println!("Words {intersection:?}");
        intersection = intersection.intersection(&word_set).cloned().collect();
    }
    Vec::from_iter(intersection)
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

    let items: Vec<_> = potentially_similar_items
        .iter_all()
        .filter_map(|(k, v)| {
            println!(
                "Similar items {:?}",
                v.iter().map(|v| v.0.to_string()).collect::<Vec<_>>()
            );
            let words: Vec<Vec<String>> = v
                .iter()
                .map(|v| break_into_words(&v.0.to_string()))
                .collect();

            let common_words = common_words(&words);
            println!("Longest prefix {common_words:?}");

            if let Some((_i, s)) = v.first() {
                let mut new_struct = (**s).clone();
                new_struct.attrs = s
                    .attrs
                    .iter()
                    .filter(|&a| {
                        a.meta.path().get_ident() != Some(&Ident::new("doc", Span::call_site()))
                    })
                    .cloned()
                    .collect();
                new_struct.fields = s.fields.clone();
                new_struct.fields.iter_mut().for_each(|f| {
                    f.attrs = f
                        .attrs
                        .clone()
                        .into_iter()
                        .filter(|a| {
                            a.meta.path().get_ident() != Some(&Ident::new("doc", Span::call_site()))
                        })
                        .collect::<Vec<_>>()
                });

                new_struct.ident = Ident::new(
                    &format!("Shared{}", common_words.iter().cloned().collect::<String>()),
                    Span::call_site(),
                );
                Some(Item::Struct(new_struct))
            } else {
                None
            }
        })
        .collect();

    let out = prettyplease::unparse(&File {
        shebang: None,
        attrs: vec![],
        items,
    });

    println!("\n\n\n Out code {out}");

    // Debug impl is available if Syn is built with "extra-traits" feature.
    //println!("{:#?}", syntax);
}

#[cfg(test)]
mod tests {
    use crate::break_into_words;

    #[test]
    fn test_word_breaking() {
        let expected_words = [
            "GRPC", "Route", "Rules", "Backend", "Refs", "Filters", "Request", "Mirror", "Backend",
            "Ref",
        ];
        let words = break_into_words("GRPCRouteRulesBackendRefsFiltersRequestMirrorBackendRef");
        assert_eq!(expected_words.to_vec(), words);

        let expected_words = [
            "GRPC", "Route", "Rules", "Backend", "Refs", "Filters", "Request", "HTTPS", "Mirror",
            "Backend", "Ref",
        ];
        let words =
            break_into_words("GRPCRouteRulesBackendRefsFiltersRequestHTTPSMirrorBackendRef");
        assert_eq!(expected_words.to_vec(), words);

        let expected_words = ["f", "RP"];
        let words = break_into_words("fRP");
        assert_eq!(expected_words.to_vec(), words);
    }
}

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use std::fs;
use std::io::Write;

use clap::Parser;
use clap::command;
use itertools::Itertools;
use log::debug;
use log::info;
use multimap::MultiMap;
use proc_macro2::{Ident, Span};
use syn::File;
use syn::Item;
use syn::ItemStruct;
use syn::PathSegment;
use syn::Type;
use syn::visit::{self, Visit};
use syn::visit_mut;
use syn::visit_mut::VisitMut;

const SHARED_TYPES_FILE_PREAMBLE: &str = "#[allow(unused_imports)]
mod prelude {
    pub use k8s_openapi::apimachinery::pkg::apis::meta::v1::Condition;
    pub use kube::CustomResource;
    pub use schemars::JsonSchema;
    pub use serde::{Deserialize, Serialize};
    pub use std::collections::BTreeMap;
}
use self::prelude::*;";

const SHARED_TYPES_USE_PREAMBLE: &str = "use crate::shared_types::*;\n\n";

struct StructVisitor<'ast> {
    name: String,
    structs: Vec<&'ast ItemStruct>,
}

struct StructRenamer {
    names: BTreeMap<String, String>,
}

fn rewrite_ident(path: &mut PathSegment, names: &BTreeMap<String, String>) {
    if path.arguments.is_empty() {
        let ident = &path.ident;
        if let Some(new_name) = names.get(&ident.to_string()) {
            path.ident = Ident::new(new_name, Span::call_site());
        }
    } else {
        match path.arguments {
            syn::PathArguments::None => {}
            syn::PathArguments::AngleBracketed(ref mut angle_bracketed_generic_arguments) => {
                for a in angle_bracketed_generic_arguments.args.iter_mut() {
                    if let syn::GenericArgument::Type(Type::Path(path)) = a {
                        for s in path.path.segments.iter_mut() {
                            rewrite_ident(s, names);
                        }
                    }
                }
            }
            syn::PathArguments::Parenthesized(_) => {}
        }
    }
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
        debug!("Visiting Struct name == {}", node.ident);
        //debug!("Visiting Struct name == {:#?}", node);
        let mut is_simple_leaf = true;
        node.fields.iter().for_each(|f| match &f.ty {
            Type::Path(path_type) => {
                debug!(
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
        debug!("Visiting Struct name == {} {is_simple_leaf}", node.ident);
        if is_simple_leaf {
            self.structs.push(node);
        }
        visit::visit_item_struct(self, node);
    }
}

impl VisitMut for StructRenamer {
    fn visit_item_struct_mut(&mut self, node: &mut ItemStruct) {
        debug!("Renaming Struct name == {}", node.ident);

        node.fields.iter_mut().for_each(|f| {
            let ty = f.ty.clone();
            if let Type::Path(path_type) = &mut f.ty {
                debug!(
                    "\twith field name = {:?} \n\t\tfield type = {:?}",
                    f.ident, ty
                );

                for segment in &mut path_type.path.segments {
                    rewrite_ident(segment, &self.names);
                }
            }
        });

        visit_mut::visit_item_struct_mut(self, node);
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
        intersection = intersection.intersection(&word_set).cloned().collect();
    }
    Vec::from_iter(intersection)
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(long)]
    apis_dir: String,

    #[arg(long)]
    out_dir: String,
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    simple_logger::init_with_env().unwrap();
    let Args { apis_dir, out_dir } = Args::parse();

    let Ok(_) = fs::exists(out_dir.clone()) else {
        return Err("our dir doesn't exist".into());
    };

    let mut visitors = vec![];

    for dir_entry in fs::read_dir(apis_dir).unwrap() {
        let Ok(dir_entry) = dir_entry else {
            continue;
        };

        if let Ok(name) = dir_entry.file_name().into_string() {
            if name.ends_with(".rs") {
                info!("Adding file {:?}", dir_entry.path());
                if let Ok(api_file) = fs::read_to_string(dir_entry.path()) {
                    if let Ok(syntaxt_file) = syn::parse_file(&api_file) {
                        let visitor = StructVisitor {
                            name,
                            structs: Vec::new(),
                        };
                        visitors.push((visitor, syntaxt_file));
                    }
                }
            }
        }
    }

    let mut potentially_similar_items = MultiMap::new();

    let visitors: Vec<_> = visitors
        .into_iter()
        .map(|(mut visitor, file)| {
            visitor.visit_file(&file);
            visitor.structs.into_iter().for_each(|i| {
                potentially_similar_items.insert(i.fields.clone(), (i.ident.clone(), (*i).clone()));
            });
            (visitor.name, file)
        })
        .collect();

    let items: Vec<_> = potentially_similar_items
        .iter_all()
        .filter(|(_k, v)| v.len() > 1)
        .filter_map(|(_k, v)| {
            info!(
                "Potentially similar structs {:#?}",
                v.iter().map(|(i, _)| i.to_string()).collect::<Vec<_>>()
            );
            let mapped_type_names = v.iter().map(|v| v.0.to_string()).collect::<Vec<_>>();

            let words: Vec<Vec<String>> = v
                .iter()
                .map(|v| break_into_words(&v.0.to_string()))
                .collect();

            let common_words = common_words(&words);

            if let Some((_i, s)) = v.first() {
                let mut new_struct = s.clone();
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
                let mut mapped = BTreeMap::new();
                for mapped_type_name in mapped_type_names {
                    mapped.insert(mapped_type_name, new_struct.ident.to_string().to_owned());
                }

                info!("Mapped types = {:#?}", &mapped);
                if mapped.keys().len() < 2 {
                    None
                } else {
                    Some((mapped, Item::Struct(new_struct)))
                }
            } else {
                None
            }
        })
        .collect();

    let (mapped, items): (Vec<BTreeMap<String, String>>, Vec<Item>) = items.into_iter().unzip();
    let mapped: BTreeMap<String, String> = mapped.into_iter().flatten().collect();

    let mut renaming_visitor = StructRenamer { names: mapped };

    let unparsed_files: Vec<(String, String)> = visitors
        .into_iter()
        .map(|(name, mut f)| {
            renaming_visitor.visit_file_mut(&mut f);
            (name, prettyplease::unparse(&f))
        })
        .collect();

    let out = prettyplease::unparse(&File {
        shebang: None,
        attrs: vec![],
        items,
    });

    let output_path = std::path::Path::new(&out_dir);

    if output_path.is_dir() && output_path.exists() {
        for (name, content) in unparsed_files {
            let mut out_file = std::fs::File::create(output_path.join(name))?;
            out_file
                .write_all((SHARED_TYPES_USE_PREAMBLE.to_owned() + &content).as_bytes())
                .unwrap();
        }

        let mut shared_out_file = std::fs::File::create(output_path.join("shared_types.rs"))?;
        let out = SHARED_TYPES_FILE_PREAMBLE.to_owned() + "\n\n\n" + &out;
        shared_out_file.write_all(out.as_bytes())?;
        Ok(())
    } else {
        Err("Make sure that output path is a folder and tha it exists".into())
    }
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

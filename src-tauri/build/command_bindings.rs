//! Read actual Tauri command signatures; generate Rust that checks every boundary type with TS.
//! No separate command schema is maintained. Unsupported signature forms fail the build.
use quote::ToTokens;
use std::{collections::BTreeSet, env, fs, path::PathBuf};
use syn::{visit::Visit, FnArg, GenericArgument, Item, Pat, PathArguments, ReturnType, Type};

fn type_name(ty: &Type) -> Option<&str> {
    // Used only for Tauri-injected arguments, which never cross IPC.
    match ty {
        Type::Path(path) => path
            .path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .and_then(|name| match name.as_str() {
                "State" => Some("State"),
                "AppHandle" => Some("AppHandle"),
                _ => None,
            }),
        _ => None,
    }
}

fn success_type(output: &ReturnType) -> String {
    let ReturnType::Type(_, ty) = output else {
        return "\"void\".to_owned()".into();
    };
    let Type::Path(path) = ty.as_ref() else {
        panic!("IPC results must be Result<T, String>")
    };
    let segment = path.path.segments.last().unwrap();
    if segment.ident != "Result" {
        return format!(
            "<{} as ts_rs::TS>::name(&ts_rs::Config::from_env())",
            ty.to_token_stream()
        );
    }
    let PathArguments::AngleBracketed(args) = &segment.arguments else {
        panic!("missing Result types")
    };
    let GenericArgument::Type(result) = &args.args[0] else {
        panic!("invalid Result")
    };
    assert_eq!(
        args.args[1].to_token_stream().to_string(),
        "String",
        "IPC error type must be String"
    );
    if matches!(result, Type::Tuple(t) if t.elems.is_empty()) {
        "\"void\".to_owned()".into()
    } else {
        format!(
            "<{} as ts_rs::TS>::name(&ts_rs::Config::from_env())",
            result.to_token_stream()
        )
    }
}

#[derive(Default)]
struct HandlerNames(Vec<String>);
impl<'ast> Visit<'ast> for HandlerNames {
    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        if node
            .path
            .segments
            .last()
            .is_some_and(|s| s.ident == "generate_handler")
        {
            assert!(
                self.0.is_empty(),
                "Keep one native command registration table"
            );
            self.0 = node
                .tokens
                .to_string()
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect();
        }
        syn::visit::visit_macro(self, node);
    }
}

pub fn generate() {
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=build/command_bindings.rs");
    let source = fs::read_to_string("src/lib.rs").unwrap();
    let tree = syn::parse_file(&source).expect("parse native composition root");
    let mut handlers = HandlerNames::default();
    handlers.visit_file(&tree);
    let mut names = BTreeSet::new();
    let mut commands = Vec::new();
    for item in tree.items {
        let Item::Fn(function) = item else { continue };
        let Some(attribute) = function
            .attrs
            .iter()
            .find(|a| a.path().to_token_stream().to_string() == "tauri :: command")
        else {
            continue;
        };
        assert!(
            matches!(attribute.meta, syn::Meta::Path(_)),
            "Command attribute options require explicit binding-generator support"
        );
        let name = function.sig.ident.to_string();
        names.insert(name.clone());
        let mut arguments = Vec::new();
        for input in &function.sig.inputs {
            let FnArg::Typed(argument) = input else {
                panic!("IPC receiver unsupported")
            };
            if type_name(&argument.ty).is_some() {
                continue;
            }
            let Pat::Ident(ident) = argument.pat.as_ref() else {
                panic!("IPC arguments must be named")
            };
            let mut camel = String::new();
            for (index, part) in ident.ident.to_string().split('_').enumerate() {
                if index == 0 {
                    camel.push_str(part)
                } else {
                    let mut chars = part.chars();
                    if let Some(c) = chars.next() {
                        camel.extend(c.to_uppercase());
                        camel.extend(chars);
                    }
                }
            }
            let ty = argument.ty.to_token_stream();
            let optional = matches!(argument.ty.as_ref(), Type::Path(p) if p.path.segments.last().is_some_and(|s| s.ident == "Option"));
            arguments.push(format!("serde_json::json!({{\"name\": {camel:?}, \"type\": <{ty} as ts_rs::TS>::name(&ts_rs::Config::from_env()), \"optional\": {optional}}})"));
        }
        let result = success_type(&function.sig.output);
        commands.push(format!(
            "serde_json::json!({{\"name\": {name:?}, \"args\": [{}], \"result\": {result}}})",
            arguments.join(",")
        ));
    }
    assert_eq!(
        names,
        handlers.0.into_iter().collect(),
        "Every command must be registered and covered by generated bindings"
    );
    let code = format!(
        r#"
#[test]
fn export_command_bindings() {{
    let metadata = serde_json::json!({{
        "commands": [{}],
        "events": kestrel_app_core::events::bindings(),
        "speechPreferencesDefaults": kestrel_app_core::SpeechPreferences::default(),
        "movieProducerDefaults": kestrel_app_core::MovieProducerProjectSettings::default(),
        "runtimePolicyCatalog": kestrel_app_core::RUNTIME_POLICY_CATALOG,
        "externalCollaborationFormat": kestrel_app_core::EXTERNAL_COLLABORATION,
    }});
    if let Ok(path) = std::env::var("KESTREL_IPC_EXPORT_PATH") {{
        std::fs::write(path, serde_json::to_string_pretty(&metadata).unwrap()).unwrap();
    }}
}}
"#,
        commands.join(",\n")
    );
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("command_bindings.rs"),
        code,
    )
    .unwrap();
}

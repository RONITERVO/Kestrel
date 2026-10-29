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

fn success_rust_type(output: &ReturnType) -> Type {
    let ReturnType::Type(_, ty) = output else {
        return syn::parse_quote!(());
    };
    let Type::Path(path) = ty.as_ref() else {
        panic!("IPC results must be Result<T, String>")
    };
    let segment = path.path.segments.last().unwrap();
    if segment.ident != "Result" {
        return ty.as_ref().clone();
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
    result.clone()
}

fn success_type(output: &ReturnType) -> String {
    let result = success_rust_type(output);
    if matches!(&result, Type::Tuple(t) if t.elems.is_empty()) {
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
    let mut dispatch = Vec::new();
    let mut descriptions = Vec::new();
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
        let mut decoded = Vec::new();
        let mut calls = Vec::new();
        let mut keys = Vec::new();
        let mut argument_types = Vec::new();
        for input in &function.sig.inputs {
            let FnArg::Typed(argument) = input else {
                panic!("IPC receiver unsupported")
            };
            if let Some(injected) = type_name(&argument.ty) {
                calls.push(
                    if injected == "State" {
                        "app.state::<AppState>()"
                    } else {
                        "app.clone()"
                    }
                    .to_string(),
                );
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
            let variable = ident.ident.to_string();
            keys.push(format!("{camel:?}"));
            decoded.push(format!("let {variable}: {ty} = serde_json::from_value(args.get({camel:?}).cloned().unwrap_or(serde_json::Value::Null)).map_err(|e| format!(\"Invalid argument {{}} for {{}}: {{e}}\", {camel:?}, {name:?}))?;"));
            calls.push(variable);
            argument_types.push(format!("app_control::describe_type::<{ty}>(&mut types);"));
            let optional = matches!(argument.ty.as_ref(), Type::Path(p) if p.path.segments.last().is_some_and(|s| s.ident == "Option"));
            arguments.push(format!("serde_json::json!({{\"name\": {camel:?}, \"type\": <{ty} as ts_rs::TS>::name(&ts_rs::Config::from_env()), \"optional\": {optional}}})"));
        }
        let result = success_type(&function.sig.output);
        commands.push(format!(
            "serde_json::json!({{\"name\": {name:?}, \"args\": [{}], \"result\": {result}}})",
            arguments.join(",")
        ));
        let await_call = if function.sig.asyncness.is_some() {
            ".await"
        } else {
            ""
        };
        let fallible = matches!(&function.sig.output, ReturnType::Type(_, ty) if matches!(ty.as_ref(), Type::Path(p) if p.path.segments.last().is_some_and(|s| s.ident == "Result")));
        let question = if fallible { "?" } else { "" };
        let call = format!("{name}({}){await_call}{question}", calls.join(","));
        let invocation = if result == "\"void\".to_owned()" {
            format!("{call}; Ok(serde_json::Value::Null)")
        } else {
            format!("let result = {call}; serde_json::to_value(result).map_err(|e| e.to_string())")
        };
        dispatch.push(format!(
            "{name:?} => {{ app_control::validate_keys(&args, &[{}])?; {} {invocation} }}",
            keys.join(","),
            decoded.join("\n")
        ));
        let result_rust = success_rust_type(&function.sig.output);
        if !matches!(&result_rust, Type::Tuple(t) if t.elems.is_empty()) {
            let ty = result_rust.to_token_stream();
            argument_types.push(format!("app_control::describe_type::<{ty}>(&mut types);"));
        }
        let mutable = if argument_types.is_empty() {
            ""
        } else {
            "mut "
        };
        descriptions.push(format!("{name:?} => {{ let {mutable}types = std::collections::BTreeMap::new(); {} Ok((serde_json::json!([{}]), {result}, types)) }}", argument_types.join("\n"), arguments.join(",")));
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
    let dispatch_code = format!(
        r#"
fn dispatch_app_command<'a>(app: &'a AppHandle, name: &'a str, args: serde_json::Value) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<serde_json::Value, String>> + Send + 'a>> {{
    Box::pin(async move {{ match name {{ {}, _ => Err(format!("Unknown app command: {{name}}")) }} }})
}}
fn describe_app_command(name: &str) -> Result<app_control::TypeDescription, String> {{
    match name {{ {}, _ => Err(format!("Unknown app command: {{name}}")) }}
}}
const APP_COMMAND_NAMES: &[&str] = &[{}];
"#,
        dispatch.join(",\n"),
        descriptions.join(",\n"),
        names
            .iter()
            .map(|n| format!("{n:?}"))
            .collect::<Vec<_>>()
            .join(",")
    );
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("app_dispatch.rs"),
        dispatch_code,
    )
    .unwrap();
}

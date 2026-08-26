use std::path::Path;
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use base64::Engine as _;
use notify::{RecursiveMode, Watcher};
use serde_json::Value;

use crate::command_helpers::flag_value;
use crate::mcp_http_cli::{post, tool_call_body};
use crate::{Command, Flags};

pub(crate) fn set_runtime_entrypoint(file: &str, entrypoint: &str) -> Result<String, String> {
    let (name, runtime_id) = entrypoint
        .split_once('=')
        .filter(|(name, runtime_id)| is_runtime_id(name) && is_runtime_id(runtime_id))
        .ok_or("--entrypoint must be NAME=RUNTIME_ID using OPUI identifier characters")?;
    let path = Path::new(file);
    let mut document = op_runtime_ui::load_document(path).map_err(|error| error.to_string())?;
    document
        .runtime_entrypoints
        .get_or_insert_default()
        .insert(name.into(), runtime_id.into());
    op_runtime_ui::write_document(path, &document).map_err(|error| error.to_string())?;
    Ok(serde_json::json!({
        "event": "runtime_entrypoint_updated",
        "file": file,
        "name": name,
        "runtime_id": runtime_id,
    })
    .to_string())
}

pub(crate) fn apply_runtime_metadata(file: &str, spec: &str) -> Result<String, String> {
    let path = Path::new(file);
    let document = op_runtime_ui::load_document(path).map_err(|error| error.to_string())?;
    let mut value = serde_json::to_value(document).map_err(|error| error.to_string())?;
    let spec: Value = serde_json::from_slice(
        &std::fs::read(spec).map_err(|error| format!("read {spec}: {error}"))?,
    )
    .map_err(|error| format!("parse {spec}: {error}"))?;
    let nodes = spec
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or("runtime metadata spec requires a nodes array")?;
    for node in nodes {
        let name = node
            .get("name")
            .and_then(Value::as_str)
            .ok_or("runtime metadata node requires name")?;
        let runtime_id = node
            .get("runtimeId")
            .and_then(Value::as_str)
            .filter(|runtime_id| is_runtime_id(runtime_id))
            .ok_or_else(|| format!("runtime metadata node `{name}` has invalid runtimeId"))?;
        let node_type = node.get("type").and_then(Value::as_str);
        let mut visual_states = serde_json::Map::new();
        if let Some(states) = node.get("visualStates").and_then(Value::as_object) {
            for (state, target_name) in states {
                let target_name = target_name
                    .as_str()
                    .ok_or_else(|| format!("visual state `{state}` on `{name}` must be a name"))?;
                let target_runtime_id = format!("{runtime_id}.{state}");
                set_named_node_field(
                    &mut value,
                    target_name,
                    Some("frame"),
                    "runtimeId",
                    Value::String(target_runtime_id.clone()),
                )?;
                visual_states.insert(state.clone(), Value::String(target_runtime_id));
            }
        }
        set_named_node_field(
            &mut value,
            name,
            node_type,
            "runtimeId",
            Value::String(runtime_id.into()),
        )?;
        for (source, target) in [
            ("role", "role"),
            ("accessibilityLabel", "accessibilityLabel"),
            ("tabIndex", "tabIndex"),
            ("enabled", "enabled"),
        ] {
            if let Some(field) = node.get(source) {
                set_named_node_field(&mut value, name, node_type, target, field.clone())?;
            }
        }
        if !visual_states.is_empty() {
            set_named_node_field(
                &mut value,
                name,
                node_type,
                "visualStates",
                Value::Object(visual_states),
            )?;
        }
    }
    if let Some(entrypoints) = spec.get("entrypoints") {
        value["runtimeEntrypoints"] = entrypoints.clone();
    }
    let document = serde_json::from_value(value)
        .map_err(|error| format!("runtime metadata does not match the .op schema: {error}"))?;
    op_runtime_ui::write_document(path, &document).map_err(|error| error.to_string())?;
    Ok(serde_json::json!({
        "event": "runtime_metadata_updated",
        "file": file,
        "nodes": nodes.len(),
    })
    .to_string())
}

fn set_named_node_field(
    document: &mut Value,
    name: &str,
    node_type: Option<&str>,
    field: &str,
    value: Value,
) -> Result<(), String> {
    let count = count_named_nodes(document, name, node_type);
    if count != 1 {
        return Err(format!(
            "runtime metadata selector `{name}` matched {count} nodes; name/type selectors must be unique"
        ));
    }
    let node = find_named_node_mut(document, name, node_type).expect("count proved one match");
    node.insert(field.into(), value);
    Ok(())
}

fn count_named_nodes(value: &Value, name: &str, node_type: Option<&str>) -> usize {
    match value {
        Value::Object(object) => {
            usize::from(
                object.get("type").is_some()
                    && object.get("name").and_then(Value::as_str) == Some(name)
                    && node_type.is_none_or(|node_type| {
                        object.get("type").and_then(Value::as_str) == Some(node_type)
                    }),
            ) + object
                .values()
                .map(|value| count_named_nodes(value, name, node_type))
                .sum::<usize>()
        }
        Value::Array(values) => values
            .iter()
            .map(|value| count_named_nodes(value, name, node_type))
            .sum(),
        _ => 0,
    }
}

fn find_named_node_mut<'a>(
    value: &'a mut Value,
    name: &str,
    node_type: Option<&str>,
) -> Option<&'a mut serde_json::Map<String, Value>> {
    match value {
        Value::Object(object) => {
            if object.get("type").is_some()
                && object.get("name").and_then(Value::as_str) == Some(name)
                && node_type.is_none_or(|node_type| {
                    object.get("type").and_then(Value::as_str) == Some(node_type)
                })
            {
                return Some(object);
            }
            object
                .values_mut()
                .find_map(|value| find_named_node_mut(value, name, node_type))
        }
        Value::Array(values) => values
            .iter_mut()
            .find_map(|value| find_named_node_mut(value, name, node_type)),
        _ => None,
    }
}

fn is_runtime_id(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some('A'..='Z' | 'a'..='z'))
        && chars.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '/' | '-')
        })
}

pub(crate) fn map_export(flags: &Flags) -> Result<Command, String> {
    let item_id = flag_value(flags, "item");
    let selection = flags.contains_key("selection");
    if item_id.is_some() && selection {
        return Err("--item and --selection cannot be used together".into());
    }
    let format_flag = flag_value(flags, "format");
    let formats_flag = flag_value(flags, "formats");
    if let (Some(format), Some(formats)) = (&format_flag, &formats_flag) {
        if format != formats {
            return Err("--format and --formats cannot specify different values".into());
        }
    }
    let format = format_flag.or(formats_flag).unwrap_or_else(|| "png".into());
    if format == "opui" {
        if selection {
            return Err("--selection is not supported for --format opui".into());
        }
        if flag_value(flags, "scale").is_some() {
            return Err("--scale is not supported for --format opui".into());
        }
        let file = flag_value(flags, "file").ok_or("--file is required for --format opui")?;
        let output = flag_value(flags, "output").ok_or("--output is required")?;
        if !output.ends_with(".opui") {
            return Err("--output must end with .opui".into());
        }
        if flags.contains_key("raster-native") && flags.contains_key("strict") {
            return Err("--strict and --raster-native cannot be used together".into());
        }
        let raster_native = flags.contains_key("raster-native");
        if raster_native && !cfg!(feature = "opui-raster") {
            return Err("rebuild op with --features opui-raster".into());
        }
        let watch = flags.contains_key("watch");
        let debounce_ms = flag_value(flags, "debounce-ms")
            .map(|value| {
                value
                    .parse::<u64>()
                    .map_err(|_| format!("--debounce-ms must be an integer, got {value:?}"))
            })
            .transpose()?
            .unwrap_or(150);
        if debounce_ms == 0 {
            return Err("--debounce-ms must be greater than zero".into());
        }
        if flags.contains_key("debounce-ms") && !watch {
            return Err("--debounce-ms requires --watch".into());
        }
        return Ok(Command::ExportOpui {
            file,
            item_id,
            output,
            strict: flags.contains_key("strict"),
            raster_native,
            watch,
            debounce_ms,
        });
    }
    if !matches!(format.as_str(), "png" | "jpeg" | "jpg" | "webp" | "pdf") {
        return Err(format!("unsupported export format {format:?}"));
    }
    let output = flag_value(flags, "output").ok_or("--output is required")?;
    let scale = flag_value(flags, "scale");
    if let Some(value) = &scale {
        value
            .parse::<f32>()
            .map_err(|_| format!("--scale must be a number, got {value:?}"))?;
    }
    Ok(Command::Export {
        item_id,
        selection,
        output,
        format,
        scale,
    })
}

pub(crate) fn run_export_opui_watch(
    file: &str,
    output: &str,
    item_id: Option<&str>,
    strict: bool,
    raster_native: bool,
    debounce_ms: u64,
) -> Result<String, String> {
    run_export_opui_watch_for(
        file,
        output,
        item_id,
        strict,
        raster_native,
        debounce_ms,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_export_opui_watch_for(
    file: &str,
    output: &str,
    item_id: Option<&str>,
    strict: bool,
    raster_native: bool,
    debounce_ms: u64,
    max_updates: Option<usize>,
) -> Result<String, String> {
    println!(
        "{}",
        run_export_opui(file, output, item_id, strict, raster_native)?
    );
    let source = std::path::absolute(file)
        .map_err(|error| format!("resolve watched source {file}: {error}"))?;
    let parent = source
        .parent()
        .ok_or_else(|| format!("watched source has no parent: {}", source.display()))?;
    let (send, receive) = std::sync::mpsc::channel();
    let mut watcher = notify::recommended_watcher(send)
        .map_err(|error| format!("create file watcher: {error}"))?;
    watcher
        .watch(parent, RecursiveMode::NonRecursive)
        .map_err(|error| format!("watch {}: {error}", parent.display()))?;
    let debounce = Duration::from_millis(debounce_ms);
    let mut updates = 0;

    loop {
        let event = match receive.recv() {
            Ok(Ok(event)) => event,
            Ok(Err(error)) => {
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "event": "watch_error",
                        "source": source,
                        "error": error.to_string(),
                    })
                );
                continue;
            }
            Err(error) => return Err(format!("file watcher disconnected: {error}")),
        };
        let mut changed = event.paths.iter().any(|path| path == &source);
        loop {
            match receive.recv_timeout(debounce) {
                Ok(Ok(event)) => {
                    changed |= event.paths.iter().any(|path| path == &source);
                }
                Ok(Err(error)) => eprintln!(
                    "{}",
                    serde_json::json!({
                        "event": "watch_error",
                        "source": source,
                        "error": error.to_string(),
                    })
                ),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    return Err("file watcher disconnected".into());
                }
            }
        }
        if !changed {
            continue;
        }
        match run_export_opui(file, output, item_id, strict, raster_native) {
            Ok(message) => {
                println!("{message}");
                updates += 1;
                if max_updates == Some(updates) {
                    return Ok(serde_json::json!({
                        "event": "watch_complete",
                        "updates": updates,
                    })
                    .to_string());
                }
            }
            Err(error) => eprintln!(
                "{}",
                serde_json::json!({
                    "event": "export_failed",
                    "source": source,
                    "output": output,
                    "error": error,
                    "last_good_retained": true,
                })
            ),
        }
    }
}

pub(crate) fn run_export_opui(
    file: &str,
    output: &str,
    item_id: Option<&str>,
    strict: bool,
    raster_native: bool,
) -> Result<String, String> {
    let output_path = Path::new(output);
    #[allow(unused_mut)]
    let mut result = op_runtime_ui::prepare_export(Path::new(file), output_path, item_id, strict)
        .map_err(|e| e.to_string())?;
    if raster_native {
        #[cfg(feature = "opui-raster")]
        crate::opui_raster::apply(file, &mut result)?;
        #[cfg(not(feature = "opui-raster"))]
        return Err("rebuild op with --features opui-raster".into());
    }
    op_runtime_ui::write_package(output_path, &result).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "output": output,
        "format": "opui",
        "itemId": item_id.unwrap_or(""),
    })
    .to_string())
}

pub(crate) fn run_export(
    port: u16,
    item_id: Option<&str>,
    output: &str,
    format: &str,
    scale: Option<&str>,
) -> Result<String, String> {
    let mut arguments = serde_json::Map::new();
    if let Some(item_id) = item_id {
        arguments.insert("itemId".into(), Value::String(item_id.into()));
    }
    arguments.insert("format".into(), Value::String(format.into()));
    if let Some(scale) = scale {
        let scale = scale
            .parse::<f64>()
            .map_err(|_| format!("--scale must be a number, got {scale:?}"))?;
        arguments.insert("scale".into(), Value::from(scale));
    }
    let response = post(
        port,
        &tool_call_body("export_item", &Value::Object(arguments).to_string()),
    )?;
    write_export_response(&response, Path::new(output))
}

pub(crate) fn write_export_response(response: &str, output: &Path) -> Result<String, String> {
    let value: Value = serde_json::from_str(response)
        .map_err(|error| format!("export_item returned invalid JSON: {error}"))?;
    let encoded = value
        .get("bytes_base64")
        .and_then(Value::as_str)
        .ok_or("export_item response is missing bytes_base64")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|error| format!("export_item returned invalid Base64: {error}"))?;
    std::fs::write(output, bytes)
        .map_err(|error| format!("cannot write export to {}: {error}", output.display()))?;

    Ok(serde_json::json!({
        "output": output.to_string_lossy(),
        "itemId": value.get("itemId").and_then(Value::as_str).unwrap_or(""),
        "itemType": value.get("itemType").and_then(Value::as_str).unwrap_or(""),
        "format": value.get("format").and_then(Value::as_str).unwrap_or(""),
    })
    .to_string())
}

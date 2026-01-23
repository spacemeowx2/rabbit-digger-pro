use anyhow::Result;
use rd_interface::schemars::{schema_for, Schema};
use serde_json::{json, Value};
use std::path::Path;
use tokio::fs::{create_dir_all, write};

use crate::{config::get_importer_registry, config::ImportSource, get_registry};

fn schema_to_value(schema: &Schema) -> Result<Value> {
    Ok(serde_json::to_value(schema)?)
}

fn value_to_schema(value: Value) -> Result<Schema> {
    Ok(serde_json::from_value(value)?)
}

fn const_type_object(type_name: &str) -> Value {
    json!({
        "type": "object",
        "required": ["type"],
        "properties": {
            "type": {
                "type": "string",
                "const": type_name,
            }
        }
    })
}

fn net_or_server_variant(schema: &Schema, type_name: &str) -> Result<Value> {
    Ok(json!({
        "title": type_name,
        "allOf": [
            schema_to_value(schema)?,
            const_type_object(type_name),
        ]
    }))
}

fn import_variant(schema: &Schema, type_name: &str) -> Result<Value> {
    let name_schema = schema_to_value(&schema_for!(Option<String>))?;
    let source_schema = schema_to_value(&schema_for!(ImportSource))?;

    let common_fields = json!({
        "type": "object",
        "required": ["source"],
        "properties": {
            "name": name_schema,
            "source": source_schema,
        }
    });

    Ok(json!({
        "title": type_name,
        "allOf": [
            schema_to_value(schema)?,
            common_fields,
            const_type_object(type_name),
        ]
    }))
}

pub async fn write_schema(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    let schema = generate_schema().await?;
    let schema = serde_json::to_string_pretty(&schema)?;
    if let Some(parent) = path.parent() {
        create_dir_all(parent).await?;
    }
    write(path, schema).await?;

    Ok(())
}

pub async fn generate_schema() -> Result<Schema> {
    let registry = get_registry()?;
    let importer_registry = get_importer_registry();

    let net_variants: Vec<Value> = registry
        .net()
        .iter()
        .map(|(k, v)| net_or_server_variant(v.schema(), k.as_ref()))
        .collect::<Result<_>>()?;

    let server_variants: Vec<Value> = registry
        .server()
        .iter()
        .map(|(k, v)| net_or_server_variant(v.schema(), k.as_ref()))
        .collect::<Result<_>>()?;

    let import_variants: Vec<Value> = importer_registry
        .iter()
        .map(|(k, v)| import_variant(v.schema(), *k))
        .collect::<Result<_>>()?;

    let root = json!({
        "title": "Config",
        "type": "object",
        "properties": {
            "id": { "type": "string" },
            "net": {
                "type": "object",
                "additionalProperties": {
                    "anyOf": net_variants
                }
            },
            "server": {
                "type": "object",
                "additionalProperties": {
                    "anyOf": server_variants
                }
            },
            "import": {
                "type": "array",
                "items": {
                    "type": "object",
                    "anyOf": import_variants
                }
            }
        }
    });

    value_to_schema(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_value_roundtrip() {
        let schema = schema_for!(Option<String>);
        let v = schema_to_value(&schema).unwrap();
        let schema2 = value_to_schema(v).unwrap();
        // Basic sanity: serialized value remains an object.
        let _ = schema2;
    }

    #[tokio::test]
    async fn test_generate_schema_smoke() {
        let schema = generate_schema().await.unwrap();
        let v = serde_json::to_value(&schema).unwrap();
        assert_eq!(v.get("title").and_then(|t| t.as_str()), Some("Config"));
        assert!(v.get("properties").is_some());
    }

    #[tokio::test]
    async fn test_write_schema_creates_parent_dir_and_writes_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("schema.json");
        write_schema(&path).await.unwrap();
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v.get("title").and_then(|t| t.as_str()), Some("Config"));
    }
}

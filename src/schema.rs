use anyhow::Result;
use rabbit_digger::rd_interface::schemars::{
    schema::{
        InstanceType, Metadata, ObjectValidation, RootSchema, Schema, SchemaObject,
        SubschemaValidation,
    },
    visit::{visit_root_schema, visit_schema_object, Visitor},
};
use rd_interface::schemars::schema_for;
use serde_json::Value;
use std::iter::FromIterator;
use std::{collections::BTreeMap, path::Path};
use tokio::fs::{create_dir_all, write};

use crate::{
    config::{get_importer_registry, Import, ImportSource},
    get_registry,
};

fn record(value_schema: Schema) -> Schema {
    let mut schema = SchemaObject::default();
    schema.object().additional_properties = Some(Box::new(value_schema));
    schema.into()
}

fn array(value_schema: Schema) -> Schema {
    let mut schema = SchemaObject::default();
    schema.array().items = Some(value_schema.into());
    schema.into()
}

fn any_of_schema(any_of: Vec<SchemaObject>) -> Schema {
    SchemaObject {
        instance_type: Some(InstanceType::Object.into()),
        subschemas: Some(
            SubschemaValidation {
                any_of: Some(any_of.into_iter().map(Into::into).collect()),
                ..Default::default()
            }
            .into(),
        ),
        ..Default::default()
    }
    .into()
}

fn append_type(schema: &RootSchema, type_name: &str) -> RootSchema {
    let mut schema = schema.clone();
    schema.schema.object().properties.insert(
        "type".to_string(),
        SchemaObject {
            instance_type: Some(InstanceType::String.into()),
            const_value: Some(Value::String(type_name.to_string())),
            ..Default::default()
        }
        .into(),
    );
    schema.schema.object().required.insert("type".to_string());
    schema
}

// add prefix to all $ref
struct PrefixVisitor(String);
impl PrefixVisitor {
    fn prefix(&self, key: &str) -> String {
        if ["Net"].contains(&key) {
            return key.to_string();
        }
        format!("{}{}", self.0, key)
    }
}
impl Visitor for PrefixVisitor {
    fn visit_schema_object(&mut self, schema: &mut SchemaObject) {
        if let Some(ref mut reference) = schema.reference {
            let r = reference
                .strip_prefix("#/definitions/")
                .unwrap_or(reference);
            *reference = format!("#/definitions/{}", self.prefix(r));
        }
        visit_schema_object(self, schema)
    }
    fn visit_root_schema(&mut self, root: &mut RootSchema) {
        root.definitions = root
            .definitions
            .clone()
            .into_iter()
            .map(|(k, v)| (self.prefix(&k), v))
            .collect();
        visit_root_schema(self, root)
    }
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

fn merge_config<'a>(
    prefix: &str,
    root: &mut RootSchema,
    iter: impl Iterator<Item = (&'a str, &'a RootSchema)>,
) -> Vec<SchemaObject> {
    let mut schemas: Vec<SchemaObject> = Vec::new();

    for (id, schema) in iter {
        let mut schema = append_type(schema, id);
        let mut visitor = PrefixVisitor(format!("{prefix}_{id}_"));

        visitor.visit_root_schema(&mut schema);
        schemas.push(schema.schema);
        root.definitions.extend(schema.definitions);
    }

    schemas
}

pub async fn generate_schema() -> Result<RootSchema> {
    let registry = get_registry()?;
    let importer_registry = get_importer_registry();

    let mut root: RootSchema = RootSchema::default();

    let net_schema = any_of_schema(merge_config(
        "net",
        &mut root,
        registry.net().iter().map(|(k, v)| (k.as_ref(), v.schema())),
    ));
    let server_schema = any_of_schema(merge_config(
        "server",
        &mut root,
        registry
            .server()
            .iter()
            .map(|(k, v)| (k.as_ref(), v.schema())),
    ));
    let import_schema = any_of_schema(
        merge_config(
            "import",
            &mut root,
            importer_registry.iter().map(|(k, v)| (*k, v.schema())),
        )
        .into_iter()
        .map(Import::append_schema)
        .collect(),
    );

    let string_schema = SchemaObject {
        instance_type: Some(InstanceType::String.into()),
        ..Default::default()
    }
    .into();

    root.definitions.insert("Net".to_string(), net_schema);
    root.definitions.insert("Server".to_string(), server_schema);

    let source_schema = schema_for!(ImportSource);
    root.definitions.extend(source_schema.definitions);

    root.schema = SchemaObject {
        instance_type: Some(InstanceType::Object.into()),
        metadata: Some(
            Metadata {
                title: Some("Config".to_string()),
                ..Default::default()
            }
            .into(),
        ),
        object: Some(
            ObjectValidation {
                properties: BTreeMap::from_iter([
                    ("id".to_string(), string_schema),
                    (
                        "net".to_string(),
                        record(Schema::new_ref("#/definitions/Net".into())),
                    ),
                    (
                        "server".to_string(),
                        record(Schema::new_ref("#/definitions/Server".into())),
                    ),
                    ("import".to_string(), array(import_schema)),
                ]),
                ..Default::default()
            }
            .into(),
        ),
        ..Default::default()
    };

    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_schema() {
        let schema = SchemaObject::default();
        let result = record(schema.into());
        assert!(matches!(result, Schema::Object(_)));
    }

    #[test]
    fn test_array_schema() {
        let schema = SchemaObject::default();
        let result = array(schema.into());
        assert!(matches!(result, Schema::Object(_)));
    }

    #[test]
    fn test_any_of_schema() {
        let schemas = vec![SchemaObject::default()];
        let result = any_of_schema(schemas);
        assert!(matches!(result, Schema::Object(_)));
    }

    #[test]
    fn test_append_type() {
        let mut schema = RootSchema::default();
        schema.schema = SchemaObject::default();
        let mut result = append_type(&schema, "test_type");
        assert!(result
            .schema
            .object()
            .required
            .contains(&"type".to_string()));
    }

    #[test]
    fn test_prefix_visitor_prefix() {
        let visitor = PrefixVisitor("prefix_".to_string());
        assert_eq!(visitor.prefix("Net"), "Net");
        assert_eq!(visitor.prefix("Other"), "prefix_Other");
    }

    #[tokio::test]
    async fn test_write_schema() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("schema.json");
        let result = write_schema(&path).await;
        assert!(result.is_ok());
        assert!(path.exists());
    }

    #[tokio::test]
    async fn test_generate_schema() {
        let result = generate_schema().await;
        assert!(result.is_ok());
        let schema = result.unwrap();
        assert!(schema.definitions.contains_key("Net"));
        assert!(schema.definitions.contains_key("Server"));
    }
}

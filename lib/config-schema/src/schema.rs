use std::{fs::File, io::BufReader, path::Path};

use anyhow::Result;
use once_cell::sync::OnceCell;
use schemars::schema::{InstanceType, RootSchema, Schema, SchemaObject, SingleOrVec};
use serde_json::Value;
use snafu::Snafu;
use vector_config_common::{attributes::CustomAttribute, constants::ComponentType};

fn schema_to_simple_schema<'a>(schema: &'a Schema) -> SimpleSchema<'a> {
    static TRUE_SCHEMA_OBJECT: OnceCell<SchemaObject> = OnceCell::new();
    static FALSE_SCHEMA_OBJECT: OnceCell<SchemaObject> = OnceCell::new();

    let schema_object = match schema {
        Schema::Bool(bool) => {
            if *bool {
                TRUE_SCHEMA_OBJECT.get_or_init(|| Schema::Bool(true).into_object())
            } else {
                FALSE_SCHEMA_OBJECT.get_or_init(|| Schema::Bool(false).into_object())
            }
        }
        Schema::Object(object) => object,
    };

    SimpleSchema {
        schema: schema_object,
    }
}

#[derive(Debug, Snafu)]
pub enum QueryError {
    #[snafu(display("no schemas matched the query"))]
    NoMatches,
    #[snafu(display("multiple schemas matched the query ({len})"))]
    MultipleMatches { len: usize },
    #[snafu(display("found matching attribute but was not a flag"))]
    AttributeNotFlag,
    #[snafu(display(
        "found matching attribute but expected single value; multiple values present"
    ))]
    AttributeMultipleValues,
}

#[derive(Debug, Snafu)]
pub enum SchemaError {
    #[snafu(display("invalid component schema: {pointer}: {reason}"))]
    InvalidComponentSchema {
        pointer: &'static str,
        reason: String,
    },
}

pub struct SchemaQuerier {
    schema: RootSchema,
}

impl SchemaQuerier {
    /// Creates a `SchemaQuerier` based on the schema file located at `schema_path`.
    ///
    /// # Errors
    ///
    /// If no file exists at the given schema path, or there is an I/O error during loading the file
    /// (permissions, etc), then an error variant will be returned.
    ///
    /// If the file does not contain valid JSON, or cannot be deserialized as a schema, then an error
    /// variant will be returned.
    pub fn from_schema<P: AsRef<Path>>(schema_path: P) -> Result<Self> {
        let reader = File::open(schema_path).map(BufReader::new)?;

        let schema = serde_json::from_reader(reader)?;

        Ok(Self { schema })
    }

    pub fn query(&self) -> SchemaQueryBuilder<'_> {
        SchemaQueryBuilder::from_schema(&self.schema)
    }
}

/// A query builder for querying against a root schema.
///
/// All constraints are applied in a boolean AND fashion.
pub struct SchemaQueryBuilder<'a> {
    schema: &'a RootSchema,
    attributes: Vec<CustomAttribute>,
}

impl<'a> SchemaQueryBuilder<'a> {
    fn from_schema(schema: &'a RootSchema) -> Self {
        Self {
            schema,
            attributes: Vec::new(),
        }
    }

    /// Adds a constraint on the given custom attribute.
    ///
    /// Can be used multiple times to match schemas against multiple attributes.
    ///
    /// Custom attributes are strongly matched: a flag attribute can only match a flag attribute,
    /// not a key/value attribute, and vise versa. For key/value attributes where the attribute in
    /// the schema itself has multiple values, the schema is considered a match so long as it
    /// contains the value specified in the query.
    pub fn with_custom_attribute(mut self, custom_attribute: CustomAttribute) -> Self {
        self.attributes.push(custom_attribute);
        self
    }

    /// Executes the query, returning all matching schemas.
    pub fn run(self) -> Vec<SimpleSchema<'a>> {
        let mut matches = Vec::new();

        // Search through all defined schemas.
        'schema: for schema_definition in self.schema.definitions.values() {
            match schema_definition {
                // We don't match against boolean schemas because there's nothing to match against.
                Schema::Bool(_) => continue,
                Schema::Object(schema_object) => {
                    // If we have custom attribute matches defined, but the schema has no metadata,
                    // it's not possible for it to match, so just bail out early.
                    let has_attribute_matchers = !self.attributes.is_empty();
                    let schema_metadata = schema_object.extensions.get("_metadata");
                    if has_attribute_matchers && schema_metadata.is_none() {
                        continue 'schema;
                    }

                    if let Some(Value::Object(schema_attributes)) = schema_metadata {
                        for self_attribute in &self.attributes {
                            let attr_matched = match self_attribute {
                                CustomAttribute::Flag(key) => schema_attributes
                                    .get(key)
                                    .map(|value| matches!(value, Value::Bool(true)))
                                    .unwrap_or(false),
                                CustomAttribute::KeyValue {
                                    key,
                                    value: attr_value,
                                } => schema_attributes
                                    .get(key)
                                    .map(|value| match value {
                                        // Check string values directly.
                                        Value::String(schema_attr_value) => {
                                            schema_attr_value == attr_value
                                        }
                                        // For arrays, try and convert each item to a
                                        // string, and for the values that are strings, see
                                        // if they match.
                                        Value::Array(schema_attr_values) => {
                                            schema_attr_values.iter().any(|value| {
                                                value
                                                    .as_str()
                                                    .map(|s| s == attr_value)
                                                    .unwrap_or(false)
                                            })
                                        }
                                        _ => false,
                                    })
                                    .unwrap_or(false),
                            };

                            if !attr_matched {
                                continue 'schema;
                            }
                        }
                    }

                    matches.push(SimpleSchema {
                        schema: schema_object,
                    });
                }
            }
        }

        matches
    }

    /// Executes the query, returning a single matching schema.
    ///
    /// # Errors
    ///
    /// If no schemas match, or more than one schema matches, then an error variant will be
    /// returned.
    pub fn run_single(self) -> Result<SimpleSchema<'a>, QueryError> {
        let mut matches = self.run();
        match matches.len() {
            0 => Err(QueryError::NoMatches),
            1 => Ok(matches.remove(0)),
            len => Err(QueryError::MultipleMatches { len }),
        }
    }
}

pub enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

pub enum SchemaType<'a> {
    /// A set of subschemas in which all must match.
    ///
    /// Referred to as an `allOf` schema in JSON Schema.
    ///
    /// For a given input, the input is only valid if it is valid against all specified subschemas.
    AllOf(Vec<SimpleSchema<'a>>),

    /// A set of subschemas in which only one must match.
    ///
    /// Referred to as a `oneOf` schema in JSON Schema.
    ///
    /// For a given input, the input is only valid if it is valid against exactly one of the
    /// specified subschemas.
    OneOf(Vec<SimpleSchema<'a>>),

    /// A schema that matches a well-known, constant value.
    ///
    /// Referred to by the `const` field in JSON Schema.
    ///
    /// For a given input, the input is only valid if it matches the value specified by `const`
    /// exactly. The value can be any valid JSON value.
    Constant(&'a Value),

    /// A schema that matches one of many well-known, constant values.
    ///
    /// Referred to by the `enum` field in JSON Schema.
    ///
    /// For a given input, the input is only valid if it matches one of the values specified by
    /// `enum` exactly. The values can be any valid JSON value.
    Enum(&'a Vec<Value>),

    /// A typed schema that matches a JSON data type.
    ///
    /// Referred to by the `type` field in JSON Schema.
    ///
    /// For a given input, the input is only valid if it is the same type as one of the types
    /// specified by `type`. A schema can allow multiple data types.
    Typed(OneOrMany<InstanceType>),
}

pub trait QueryableSchema {
    fn schema_type(&self) -> SchemaType;
    fn description(&self) -> Option<&str>;
    fn title(&self) -> Option<&str>;
    fn get_attributes(&self, key: &str) -> Option<OneOrMany<CustomAttribute>>;
    fn get_attribute(&self, key: &str) -> Result<Option<CustomAttribute>, QueryError>;
    fn has_flag_attribute(&self, key: &str) -> Result<bool, QueryError>;
}

impl<'a, T> QueryableSchema for &'a T
where
    T: QueryableSchema,
{
    fn schema_type(&self) -> SchemaType {
        (*self).schema_type()
    }

    fn description(&self) -> Option<&str> {
        (*self).description()
    }

    fn title(&self) -> Option<&str> {
        (*self).title()
    }

    fn get_attributes(&self, key: &str) -> Option<OneOrMany<CustomAttribute>> {
        (*self).get_attributes(key)
    }

    fn get_attribute(&self, key: &str) -> Result<Option<CustomAttribute>, QueryError> {
        (*self).get_attribute(key)
    }

    fn has_flag_attribute(&self, key: &str) -> Result<bool, QueryError> {
        (*self).has_flag_attribute(key)
    }
}

impl<'a> QueryableSchema for &'a SchemaObject {
    fn schema_type(&self) -> SchemaType {
        // TODO: Technically speaking, it is allowed to use the "X of" schema types in conjunction
        // with other schema types i.e. `allOf` in conjunction with specifying a `type`.
        //
        // Right now, the configuration schema codegen should not actually be emitting anything like
        // this, so our logic below is written against what we generate, not against what is
        // technically possible. This _may_ need to change in the future if we end up using any "X
        // of" schema composition mechanisms for richer validation (i.e. sticking special validation
        // logic in various subschemas under `allOf`, while defining the main data schema via
        // `type`, etc.)
        if let Some(subschemas) = self.subschemas.as_ref() {
            // Of all the possible "subschema" validation mechanism, we only support `allOf` and
            // `oneOf`, based on what the configuration schema codegen will spit out.
            if let Some(all_of) = subschemas.all_of.as_ref() {
                return SchemaType::AllOf(all_of.iter().map(schema_to_simple_schema).collect());
            } else if let Some(one_of) = subschemas.one_of.as_ref() {
                return SchemaType::OneOf(one_of.iter().map(schema_to_simple_schema).collect());
            } else {
                panic!("Encountered schema with subschema validation that wasn't one of the supported types: allOf, oneOf.");
            }
        }

        if let Some(instance_types) = self.instance_type.as_ref() {
            return match instance_types {
                SingleOrVec::Single(single) => SchemaType::Typed(OneOrMany::One(*single.clone())),
                SingleOrVec::Vec(many) => SchemaType::Typed(OneOrMany::Many(many.clone())),
            };
        }

        if let Some(const_value) = self.const_value.as_ref() {
            return SchemaType::Constant(const_value);
        }

        if let Some(enum_values) = self.enum_values.as_ref() {
            return SchemaType::Enum(enum_values);
        }

        panic!("Schema type was not able to be detected!");
    }

    fn description(&self) -> Option<&str> {
        self.metadata
            .as_ref()
            .and_then(|metadata| metadata.description.as_ref().map(|s| s.as_str()))
    }

    fn title(&self) -> Option<&str> {
        self.metadata
            .as_ref()
            .and_then(|metadata| metadata.title.as_ref().map(|s| s.as_str()))
    }

    fn get_attributes(&self, key: &str) -> Option<OneOrMany<CustomAttribute>> {
        self.extensions.get("_metadata")
            .map(|metadata| match metadata {
                Value::Object(attributes) => attributes,
                _ => panic!("Found metadata extension in schema that was not of type 'object'."),
            })
            .and_then(|attributes| attributes.get(key))
            .map(|attribute| match attribute {
                Value::Bool(b) => match b {
                    true => OneOrMany::One(CustomAttribute::flag(key)),
                    false => panic!("Custom attribute flags should never be false."),
                },
                Value::String(s) => OneOrMany::One(CustomAttribute::kv(key, s)),
                Value::Array(values) => {
                    let mapped = values.iter()
                        .map(|value| if let Value::String(s) = value {
                            CustomAttribute::kv(key, s)
                        } else {
                            panic!("Custom attribute key/value pair had array of values with a non-string value.")
                        })
                        .collect();
                    OneOrMany::Many(mapped)
                },
                _ => panic!("Custom attribute had unexpected non-flag/non-KV value."),
            })
    }

    fn get_attribute(&self, key: &str) -> Result<Option<CustomAttribute>, QueryError> {
        self.get_attributes(key)
            .map(|attrs| match attrs {
                OneOrMany::One(attr) => Ok(attr),
                OneOrMany::Many(_) => Err(QueryError::AttributeMultipleValues),
            })
            .transpose()
    }

    fn has_flag_attribute(&self, key: &str) -> Result<bool, QueryError> {
        self.get_attribute(key)
            .and_then(|maybe_attr| match maybe_attr {
                None => Ok(false),
                Some(attr) => {
                    if attr.is_flag() {
                        Ok(true)
                    } else {
                        Err(QueryError::AttributeNotFlag)
                    }
                }
            })
    }
}

pub struct SimpleSchema<'a> {
    schema: &'a SchemaObject,
}

impl<'a> From<&'a SchemaObject> for SimpleSchema<'a> {
    fn from(schema: &'a SchemaObject) -> Self {
        Self { schema }
    }
}

impl<'a> QueryableSchema for SimpleSchema<'a> {
    fn schema_type(&self) -> SchemaType {
        self.schema.schema_type()
    }

    fn description(&self) -> Option<&str> {
        self.schema.description()
    }

    fn title(&self) -> Option<&str> {
        self.schema.title()
    }

    fn get_attributes(&self, key: &str) -> Option<OneOrMany<CustomAttribute>> {
        self.schema.get_attributes(key)
    }

    fn get_attribute(&self, key: &str) -> Result<Option<CustomAttribute>, QueryError> {
        self.schema.get_attribute(key)
    }

    fn has_flag_attribute(&self, key: &str) -> Result<bool, QueryError> {
        self.schema.has_flag_attribute(key)
    }
}

pub struct ComponentSchema<'a> {
    schema: &'a SchemaObject,
    component_name: String,
    component_type: ComponentType,
}

impl<'a> ComponentSchema<'a> {
    pub fn component_type(&self) -> ComponentType {
        self.component_type
    }

    pub fn component_name(&self) -> &str {
        &self.component_name
    }
}

impl<'a> QueryableSchema for ComponentSchema<'a> {
    fn schema_type(&self) -> SchemaType {
        self.schema.schema_type()
    }

    fn description(&self) -> Option<&str> {
        self.schema.description()
    }

    fn title(&self) -> Option<&str> {
        self.schema.title()
    }

    fn get_attributes(&self, key: &str) -> Option<OneOrMany<CustomAttribute>> {
        self.schema.get_attributes(key)
    }

    fn get_attribute(&self, key: &str) -> Result<Option<CustomAttribute>, QueryError> {
        self.schema.get_attribute(key)
    }

    fn has_flag_attribute(&self, key: &str) -> Result<bool, QueryError> {
        self.schema.has_flag_attribute(key)
    }
}

impl<'a> TryFrom<SimpleSchema<'a>> for ComponentSchema<'a> {
    type Error = SchemaError;

    fn try_from(value: SimpleSchema<'a>) -> Result<Self, Self::Error> {
        // Component schemas must have a component type _and_ component name defined.
        let component_type = value
            .get_attribute("docs::component_type")
            .map_err(|e| SchemaError::InvalidComponentSchema {
                pointer: "docs::component_type",
                reason: e.to_string(),
            })?
            .ok_or(SchemaError::InvalidComponentSchema {
                pointer: "docs::component_type",
                reason: "attribute must be present".to_string(),
            })
            .and_then(|attr| match attr {
                CustomAttribute::Flag(_) => Err(SchemaError::InvalidComponentSchema {
                    pointer: "docs::component_type",
                    reason: "expected key/value attribute".into(),
                }),
                CustomAttribute::KeyValue { value, .. } => Ok(value),
            })
            .and_then(|s| {
                ComponentType::try_from(s.as_str()).map_err(|_| {
                    SchemaError::InvalidComponentSchema {
                        pointer: "docs::component_type",
                        reason: "value was not a valid component type".into(),
                    }
                })
            })?;

        let component_name = value
            .get_attribute("docs::component_name")
            .map_err(|e| SchemaError::InvalidComponentSchema {
                pointer: "docs::component_name",
                reason: e.to_string(),
            })?
            .ok_or(SchemaError::InvalidComponentSchema {
                pointer: "docs::component_name",
                reason: "attribute must be present".to_string(),
            })
            .and_then(|attr| match attr {
                CustomAttribute::Flag(_) => Err(SchemaError::InvalidComponentSchema {
                    pointer: "docs::component_name",
                    reason: "expected key/value attribute".into(),
                }),
                CustomAttribute::KeyValue { value, .. } => Ok(value),
            })?;

        Ok(Self {
            schema: value.schema,
            component_name,
            component_type,
        })
    }
}
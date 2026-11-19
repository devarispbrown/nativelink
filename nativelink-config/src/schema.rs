// Copyright 2026 Trace Machina, Inc. All rights reserved.
//
// Licensed under the Functional Source License, Version 1.1, Apache 2.0 Future License (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//    See LICENSE file for details
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! JSON Schema for fields that go through a `serde_utils` converter.
//!
//! schemars derives a field's schema from its Rust type, but the
//! `convert_*_with_shellexpand` deserializers accept more than the type
//! suggests: a `u64` field also takes `"${VAR}"`, a size field takes
//! `"10GB"`, a duration field takes `"1h30m"`. Without these types the
//! schema would reject configs the server accepts.
//!
//! Each type here stands in for one converter, and is attached to a field
//! with `#[cfg_attr(feature = "dev-schema", schemars(with = "..."))]`. An
//! `Option` field uses `Option<...>` around the type, which adds `null` and
//! keeps the field optional, matching the `convert_optional_*` converters.
//! The `schema_annotations_test` integration test fails if a converter field
//! is missing its annotation.
//!
//! The string branch is deliberately permissive. It can't know what an
//! environment variable expands to, and a schema that flags valid configs
//! teaches people to ignore it. The server still rejects a bad value when it
//! loads the config.
//!
//! The string converters (`convert_string_with_shellexpand` and its
//! optional and `Vec` forms) need nothing here: those fields are already
//! strings, so the derived schema is already right.

use core::marker::PhantomData;
use std::borrow::Cow;
use std::collections::HashMap;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};

use crate::cas_server::{ByteStreamConfig, OldByteStreamConfig, WithInstanceName};

const ENV_HINT: &str = "or an environment variable such as \"${NAME}\" that expands to one";

fn string_branch(what: &str) -> Schema {
    json_schema!({
        "type": "string",
        "description": format!("{what}, {ENV_HINT}."),
    })
}

fn described<T: JsonSchema>(generator: &mut SchemaGenerator, description: &str) -> Schema {
    let mut schema = generator.subschema_for::<T>();
    schema.insert("description".to_owned(), description.into());
    schema
}

macro_rules! converter_schema {
    ($(#[$doc:meta])* $name:ident, |$generator:ident| $body:expr) => {
        $(#[$doc])*
        #[derive(Debug)]
        pub struct $name<T>(PhantomData<T>);

        impl<T: JsonSchema> JsonSchema for $name<T> {
            fn inline_schema() -> bool {
                true
            }

            fn schema_name() -> Cow<'static, str> {
                format!("{}_{}", stringify!($name), T::schema_name()).into()
            }

            fn schema_id() -> Cow<'static, str> {
                format!(
                    "nativelink_config::schema::{}<{}>",
                    stringify!($name),
                    T::schema_id()
                )
                .into()
            }

            fn json_schema($generator: &mut SchemaGenerator) -> Schema {
                $body
            }
        }
    };
}

converter_schema!(
    /// `convert_numeric_with_shellexpand`: a number, or a string that
    /// expands to one.
    Numeric,
    |generator| json_schema!({
        "anyOf": [
            generator.subschema_for::<T>(),
            string_branch("A number as a string"),
        ],
    })
);

converter_schema!(
    /// `convert_data_size_with_shellexpand`: a number of bytes, or a size
    /// string such as `"10GB"`.
    DataSize,
    |generator| json_schema!({
        "anyOf": [
            described::<T>(generator, "A number of bytes."),
            string_branch("A data size such as \"500MB\", \"10GB\" or \"1.5TB\""),
        ],
    })
);

converter_schema!(
    /// `convert_duration_with_shellexpand`: a number of seconds, or a
    /// duration string such as `"1h30m"`.
    Duration,
    |generator| json_schema!({
        "anyOf": [
            described::<T>(generator, "A number of seconds."),
            string_branch("A duration such as \"30s\", \"1h30m\" or \"1d\""),
        ],
    })
);

converter_schema!(
    /// `convert_duration_with_shellexpand_and_negative`: as [`Duration`],
    /// but a negative number of seconds is allowed. Only the number form can
    /// be negative; the converter reads duration strings as unsigned.
    SignedDuration,
    |generator| json_schema!({
        "anyOf": [
            described::<T>(generator, "A number of seconds, which may be negative."),
            string_branch("A duration such as \"30s\", \"1h30m\" or \"1d\""),
        ],
    })
);

/// `convert_boolean_with_shellexpand`: a boolean, or a string that expands
/// to `true` or `false`. It doesn't accept `null`, even on an
/// `Option<bool>` field; leave the field out instead.
#[derive(Debug, Clone, Copy)]
pub struct Boolean;

impl JsonSchema for Boolean {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "Boolean".into()
    }

    fn schema_id() -> Cow<'static, str> {
        "nativelink_config::schema::Boolean".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "anyOf": [
                { "type": "boolean" },
                string_branch("\"true\" or \"false\" as a string"),
            ],
        })
    }
}

/// Old field names the server still accepts through `#[serde(alias)]`, as
/// `(old name, current name)`. schemars ignores serde aliases, so a struct
/// with an alias carries `schemars(transform = schema::add_aliases)`, which
/// adds each old name as a deprecated copy of the current field. The
/// `schema_annotations_test` integration test keeps this list complete.
pub const ALIASES: &[(&str, &str)] = &[
    ("execute_stream_heartbeat", "execute_stream_heartbeat_s"),
    (
        "persist_stream_on_disconnect_timeout",
        "persist_stream_on_disconnect_timeout_s",
    ),
    (
        "experimental_http2_keep_alive_timeout",
        "experimental_http2_keep_alive_timeout_s",
    ),
    ("max_action_timeout", "max_action_timeout_s"),
    ("max_upload_timeout", "max_upload_timeout_s"),
    ("max_download_timeout", "max_download_timeout_s"),
];

/// Adds the [`ALIASES`] that apply to this struct's properties.
pub fn add_aliases(schema: &mut Schema) {
    let Some(properties) = schema
        .as_object_mut()
        .and_then(|object| object.get_mut("properties"))
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };
    for (old, current) in ALIASES {
        let Some(mut property) = properties.get(*current).cloned() else {
            continue;
        };
        if let Some(object) = property.as_object_mut() {
            object.insert("deprecated".to_owned(), true.into());
            object.insert(
                "description".to_owned(),
                format!("Deprecated name for `{current}`.").into(),
            );
        }
        properties.entry(*old).or_insert(property);
    }
}

fn deprecated(mut schema: Schema, description: &str) -> Schema {
    schema.insert("deprecated".to_owned(), true.into());
    schema.insert("description".to_owned(), description.into());
    schema
}

converter_schema!(
    /// `backcompat::opt_vec_with_instance_name`: the list of
    /// `{ instance_name, ... }` entries, or the older map from instance name
    /// to config, which the server still reads but warns about.
    InstanceServices,
    |generator| json_schema!({
        "anyOf": [
            generator.subschema_for::<Vec<WithInstanceName<T>>>(),
            deprecated(
                generator.subschema_for::<HashMap<String, T>>(),
                "Deprecated map from instance name to config. Use the list form instead.",
            ),
        ],
    })
);

/// `backcompat::opt_bytestream`: the list of `{ instance_name, cas_store }`
/// entries, or the older `{ cas_stores: { ... } }` object, which the server
/// still reads but warns about.
#[derive(Debug, Clone, Copy)]
pub struct ByteStreamServices;

impl JsonSchema for ByteStreamServices {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "ByteStreamServices".into()
    }

    fn schema_id() -> Cow<'static, str> {
        "nativelink_config::schema::ByteStreamServices".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "anyOf": [
                generator.subschema_for::<Vec<WithInstanceName<ByteStreamConfig>>>(),
                deprecated(
                    generator.subschema_for::<OldByteStreamConfig>(),
                    "Deprecated `cas_stores` form. Use the list form instead.",
                ),
            ],
        })
    }
}

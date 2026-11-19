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

//! Every field that goes through a converter accepting more than its Rust
//! type must carry the matching `schemars(with = ...)` annotation, or the
//! published JSON Schema rejects values the server accepts (for example
//! `"10GB"` for a size, `"${VAR}"` for a number, or the older map form of a
//! service list). See `src/schema.rs`.
//!
//! This reads the crate's source as text, so it needs no extra dependency
//! and runs the same under Cargo and Bazel.

use std::fs;
use std::path::{Path, PathBuf};

/// Converter, and the start of the `schemars(with = ...)` its fields need.
const CONVERTERS: &[(&str, &str)] = &[
    ("convert_numeric_with_shellexpand", "schema::Numeric<"),
    (
        "convert_optional_numeric_with_shellexpand",
        "Option<schema::Numeric<",
    ),
    ("convert_boolean_with_shellexpand", "schema::Boolean\""),
    ("convert_data_size_with_shellexpand", "schema::DataSize<"),
    (
        "convert_optional_data_size_with_shellexpand",
        "Option<schema::DataSize<",
    ),
    ("convert_duration_with_shellexpand", "schema::Duration<"),
    (
        "convert_duration_with_shellexpand_and_negative",
        "schema::SignedDuration<",
    ),
    (
        "super::backcompat::opt_vec_with_instance_name",
        "Option<schema::InstanceServices<",
    ),
    (
        "super::backcompat::opt_bytestream",
        "Option<schema::ByteStreamServices>\"",
    ),
];

fn src_dir() -> PathBuf {
    let mut dir = Path::new(".")
        .canonicalize()
        .expect("Can canonicalize current dir");
    if dir.join("nativelink-config").exists() {
        // inside bazel
        dir = dir.join("nativelink-config");
    }
    dir.join("src")
}

fn converter_on(line: &str) -> Option<&'static (&'static str, &'static str)> {
    const KEY: &str = "deserialize_with = \"";
    let start = line.find(KEY)? + KEY.len();
    let name = &line[start..start + line[start..].find('"')?];
    CONVERTERS.iter().find(|(converter, _)| *converter == name)
}

/// Whether the struct or enum enclosing `line` derives `JsonSchema`. Types
/// that don't aren't in the schema, so their fields need no annotation.
fn enclosing_type_has_schema(lines: &[&str], line: usize) -> bool {
    let Some(item) = (0..line).rev().find(|&i| {
        let l = lines[i];
        l.starts_with("pub struct ")
            || l.starts_with("struct ")
            || l.starts_with("pub enum ")
            || l.starts_with("enum ")
    }) else {
        return false;
    };
    (0..item)
        .rev()
        .take_while(|&i| lines[i].starts_with("#[") || lines[i].starts_with("///"))
        .any(|i| lines[i].contains("JsonSchema"))
}

/// The attribute lines in front of the field that `line` belongs to, up to
/// and including the field itself.
fn field_attributes(lines: &[&str], line: usize) -> String {
    let start = (0..line)
        .rev()
        .find(|&i| {
            let l = lines[i].trim();
            l.is_empty() || l.starts_with("///") || l.ends_with('{') || l.ends_with("},")
        })
        .map_or(0, |i| i + 1);
    let end = (line..lines.len())
        .find(|&i| lines[i].trim_start().starts_with("pub "))
        .unwrap_or(line);
    lines[start..=end].join("\n")
}

#[test]
fn converter_fields_carry_schema_annotations() {
    let dir = src_dir();
    let mut checked = 0;
    let mut missing = Vec::new();

    let mut files: Vec<_> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("Failed to read {}: {e}", dir.display()))
        .map(|entry| entry.expect("Can read directory entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .collect();
    files.sort();

    for path in files {
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("Failed to read {}: {e}", path.display()));
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let Some((converter, with)) = converter_on(line) else {
                continue;
            };
            if !enclosing_type_has_schema(&lines, i) {
                continue;
            }
            checked += 1;
            let expected = format!("schemars(with = \"{with}");
            if !field_attributes(&lines, i).contains(&expected) {
                missing.push(format!(
                    "{}:{}: `{converter}` needs \
                     #[cfg_attr(feature = \"dev-schema\", {expected}...))]",
                    path.file_name().unwrap().to_string_lossy(),
                    i + 1,
                ));
            }
        }
    }

    assert!(
        checked > 100,
        "Only found {checked} converter fields; is the source scan still working?"
    );
    assert!(
        missing.is_empty(),
        "Fields whose schema would reject values the server accepts \
         (see nativelink-config/src/schema.rs):\n{}",
        missing.join("\n")
    );
}

/// The struct attribute lines above the struct or enum enclosing `line`.
fn enclosing_type_attributes(lines: &[&str], line: usize) -> String {
    let Some(item) = (0..line)
        .rev()
        .find(|&i| lines[i].starts_with("pub struct ") || lines[i].starts_with("pub enum "))
    else {
        return String::new();
    };
    (0..item)
        .rev()
        .take_while(|&i| lines[i].starts_with("#[") || lines[i].starts_with("///"))
        .map(|i| lines[i])
        .collect::<Vec<_>>()
        .join("\n")
}

/// schemars ignores `#[serde(alias)]`, so each old name must be in
/// `schema::ALIASES` and its struct must run `schema::add_aliases`.
#[test]
fn serde_aliases_are_in_the_schema() {
    let dir = src_dir();
    let schema_rs = fs::read_to_string(dir.join("schema.rs")).expect("Can read schema.rs");
    let mut checked = 0;
    let mut missing = Vec::new();

    for name in ["cas_server.rs", "stores.rs", "schedulers.rs"] {
        let text = fs::read_to_string(dir.join(name)).expect("Can read source");
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            const KEY: &str = "alias = \"";
            let Some(start) = line.find(KEY).map(|s| s + KEY.len()) else {
                continue;
            };
            let Some(len) = line[start..].find('"') else {
                continue;
            };
            let alias = &line[start..start + len];
            let attributes = enclosing_type_attributes(&lines, i);
            if !attributes.contains("JsonSchema") {
                continue;
            }
            checked += 1;
            if !schema_rs.contains(&format!("\"{alias}\"")) {
                missing.push(format!(
                    "{name}:{}: add \"{alias}\" to schema::ALIASES",
                    i + 1
                ));
            }
            if !attributes.contains("schemars(transform = schema::add_aliases)") {
                missing.push(format!(
                    "{name}:{}: the struct needs \
                     #[cfg_attr(feature = \"dev-schema\", schemars(transform = schema::add_aliases))]",
                    i + 1
                ));
            }
        }
    }

    assert!(
        checked > 0,
        "Found no serde aliases; is the source scan still working?"
    );
    assert!(
        missing.is_empty(),
        "Old field names the schema would reject:\n{}",
        missing.join("\n")
    );
}

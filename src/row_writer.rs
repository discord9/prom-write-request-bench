// Copyright 2023 Greptime Team
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use greptime_proto::v1::value::ValueData;
use greptime_proto::v1::{
    ColumnDataType, ColumnSchema, Row, RowInsertRequest, RowInsertRequests, Rows, SemanticType,
    Value,
};
use std::collections::HashMap;

pub struct TableData {
    schema: Vec<ColumnSchema>,
    rows: Vec<Row>,
    column_indexes: HashMap<String, usize>,
}

impl TableData {
    pub fn new(num_columns: usize, num_rows: usize) -> Self {
        Self {
            schema: Vec::with_capacity(num_columns),
            rows: Vec::with_capacity(num_rows),
            column_indexes: HashMap::with_capacity(num_columns),
        }
    }

    #[inline]
    pub fn num_columns(&self) -> usize {
        self.schema.len()
    }

    #[inline]
    pub fn num_rows(&self) -> usize {
        self.rows.len()
    }

    #[inline]
    pub fn alloc_one_row(&self) -> Vec<Value> {
        vec![Value { value_data: None }; self.num_columns()]
    }

    #[inline]
    pub fn add_row(&mut self, values: Vec<Value>) {
        self.rows.push(Row { values })
    }

    #[allow(dead_code)]
    pub fn columns(&self) -> &Vec<ColumnSchema> {
        &self.schema
    }

    pub fn into_schema_and_rows(self) -> (Vec<ColumnSchema>, Vec<Row>) {
        (self.schema, self.rows)
    }
}

pub struct MultiTableData {
    table_data_map: HashMap<String, TableData>,
}

impl Default for MultiTableData {
    fn default() -> Self {
        Self::new()
    }
}

impl MultiTableData {
    pub fn new() -> Self {
        Self {
            table_data_map: HashMap::new(),
        }
    }

    pub fn get_or_default_table_data(
        &mut self,
        table_name: impl ToString,
        num_columns: usize,
        num_rows: usize,
    ) -> &mut TableData {
        self.table_data_map
            .entry(table_name.to_string())
            .or_insert_with(|| TableData::new(num_columns, num_rows))
    }

    #[allow(dead_code)]
    pub fn num_tables(&self) -> usize {
        self.table_data_map.len()
    }

    /// Returns the request and number of rows in it.
    pub fn into_row_insert_requests(self) -> (RowInsertRequests, usize) {
        let mut total_rows = 0;
        let inserts = self
            .table_data_map
            .into_iter()
            .map(|(table_name, table_data)| {
                total_rows += table_data.num_rows();
                let num_columns = table_data.num_columns();
                let (schema, mut rows) = table_data.into_schema_and_rows();
                for row in &mut rows {
                    if num_columns > row.values.len() {
                        row.values.resize(num_columns, Value { value_data: None });
                    }
                }

                RowInsertRequest {
                    table_name,
                    rows: Some(Rows { schema, rows }),
                }
            })
            .collect::<Vec<_>>();
        let row_insert_requests = RowInsertRequests { inserts };

        (row_insert_requests, total_rows)
    }
}

pub fn write_tags(
    table_data: &mut TableData,
    kvs: impl Iterator<Item = (String, String)>,
    one_row: &mut Vec<Value>,
) {
    let ktv_iter = kvs.map(|(k, v)| (k, ColumnDataType::String, ValueData::StringValue(v)));
    write_by_semantic_type(table_data, SemanticType::Tag, ktv_iter, one_row)
}

pub fn write_fields(
    table_data: &mut TableData,
    fields: impl Iterator<Item = (String, ColumnDataType, ValueData)>,
    one_row: &mut Vec<Value>,
) {
    write_by_semantic_type(table_data, SemanticType::Field, fields, one_row)
}

pub fn write_f64(
    table_data: &mut TableData,
    name: impl ToString,
    value: f64,
    one_row: &mut Vec<Value>,
) {
    write_fields(
        table_data,
        std::iter::once((
            name.to_string(),
            ColumnDataType::Float64,
            ValueData::F64Value(value),
        )),
        one_row,
    )
}

fn write_by_semantic_type(
    table_data: &mut TableData,
    semantic_type: SemanticType,
    ktv_iter: impl Iterator<Item = (String, ColumnDataType, ValueData)>,
    one_row: &mut Vec<Value>,
) {
    let TableData {
        schema,
        column_indexes,
        ..
    } = table_data;

    for (name, datatype, value) in ktv_iter {
        let index = column_indexes.get(&name);
        if let Some(index) = index {
            check_schema(datatype, semantic_type, &schema[*index]);
            one_row[*index].value_data = Some(value);
        } else {
            let index = schema.len();
            schema.push(ColumnSchema {
                column_name: name.clone(),
                datatype: datatype as i32,
                semantic_type: semantic_type as i32,
                ..Default::default()
            });
            column_indexes.insert(name, index);
            one_row.push(Value {
                value_data: Some(value),
            });
        }
    }
}

pub fn write_ts_millis(
    table_data: &mut TableData,
    name: impl ToString,
    ts: Option<i64>,
    one_row: &mut Vec<Value>,
) {
    let TableData {
        schema,
        column_indexes,
        ..
    } = table_data;
    let name = name.to_string();

    let ts_val = ts.map(ValueData::TimestampMillisecondValue);
    let index = column_indexes.get(&name);
    if let Some(index) = index {
        check_schema(
            ColumnDataType::TimestampMillisecond,
            SemanticType::Timestamp,
            &schema[*index],
        );
        one_row[*index].value_data = ts_val;
    } else {
        let index = schema.len();
        schema.push(ColumnSchema {
            column_name: name.clone(),
            datatype: ColumnDataType::TimestampMillisecond as i32,
            semantic_type: SemanticType::Timestamp as i32,
            ..Default::default()
        });
        column_indexes.insert(name, index);
        one_row.push(Value { value_data: ts_val });
    }
}

#[inline]
fn check_schema(datatype: ColumnDataType, semantic_type: SemanticType, schema: &ColumnSchema) {
    assert_eq!(schema.datatype, datatype as i32,);

    assert_eq!(schema.semantic_type, semantic_type as i32);
}

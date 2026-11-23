//! A small row-oriented Parquet writer on top of arrow.

use std::fs::File;
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use arrow_array::{
    ArrayRef, BooleanArray, Float32Array, Float64Array, Int32Array, RecordBatch, StringArray,
    UInt32Array,
};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use parquet::arrow::ArrowWriter;
use parquet::basic::{Compression, ZstdLevel};
use parquet::file::properties::WriterProperties;

#[derive(Clone, Copy)]
pub enum Kind {
    Str,
    F32,
    F64,
    U32,
    I32,
    Bool,
}

enum Buf {
    Str(Vec<String>),
    F32(Vec<f32>),
    F64(Vec<f64>),
    U32(Vec<u32>),
    I32(Vec<i32>),
    Bool(Vec<bool>),
}

pub struct Table {
    schema: SchemaRef,
    bufs: Vec<Buf>,
    w: ArrowWriter<File>,
    rows: usize,
    total: usize,
}

const BATCH: usize = 262_144;

impl Table {
    pub fn create(path: &Path, cols: &[(&str, Kind)]) -> Result<Table> {
        let fields: Vec<Field> = cols
            .iter()
            .map(|(n, k)| {
                let t = match k {
                    Kind::Str => DataType::Utf8,
                    Kind::F32 => DataType::Float32,
                    Kind::F64 => DataType::Float64,
                    Kind::U32 => DataType::UInt32,
                    Kind::I32 => DataType::Int32,
                    Kind::Bool => DataType::Boolean,
                };
                Field::new(*n, t, false)
            })
            .collect();
        let schema = Arc::new(Schema::new(fields));
        let props = WriterProperties::builder()
            .set_compression(Compression::ZSTD(ZstdLevel::try_new(3)?))
            .build();
        let w = ArrowWriter::try_new(File::create(path)?, schema.clone(), Some(props))?;
        let bufs = cols
            .iter()
            .map(|(_, k)| match k {
                Kind::Str => Buf::Str(Vec::new()),
                Kind::F32 => Buf::F32(Vec::new()),
                Kind::F64 => Buf::F64(Vec::new()),
                Kind::U32 => Buf::U32(Vec::new()),
                Kind::I32 => Buf::I32(Vec::new()),
                Kind::Bool => Buf::Bool(Vec::new()),
            })
            .collect();
        Ok(Table {
            schema,
            bufs,
            w,
            rows: 0,
            total: 0,
        })
    }

    pub fn s(&mut self, i: usize, v: impl Into<String>) -> &mut Self {
        if let Buf::Str(b) = &mut self.bufs[i] {
            b.push(v.into());
        } else {
            panic!("column {i} is not a string");
        }
        self
    }
    pub fn f(&mut self, i: usize, v: f32) -> &mut Self {
        if let Buf::F32(b) = &mut self.bufs[i] {
            b.push(v);
        } else {
            panic!("column {i} is not f32");
        }
        self
    }
    pub fn d(&mut self, i: usize, v: f64) -> &mut Self {
        if let Buf::F64(b) = &mut self.bufs[i] {
            b.push(v);
        } else {
            panic!("column {i} is not f64");
        }
        self
    }
    pub fn u(&mut self, i: usize, v: u32) -> &mut Self {
        if let Buf::U32(b) = &mut self.bufs[i] {
            b.push(v);
        } else {
            panic!("column {i} is not u32");
        }
        self
    }
    pub fn n(&mut self, i: usize, v: i32) -> &mut Self {
        if let Buf::I32(b) = &mut self.bufs[i] {
            b.push(v);
        } else {
            panic!("column {i} is not i32");
        }
        self
    }
    pub fn b(&mut self, i: usize, v: bool) -> &mut Self {
        if let Buf::Bool(b) = &mut self.bufs[i] {
            b.push(v);
        } else {
            panic!("column {i} is not bool");
        }
        self
    }

    pub fn end_row(&mut self) -> Result<()> {
        self.rows += 1;
        if self.rows >= BATCH {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        if self.rows == 0 {
            return Ok(());
        }
        let arrays: Vec<ArrayRef> = self
            .bufs
            .iter_mut()
            .map(|b| -> ArrayRef {
                match b {
                    Buf::Str(v) => Arc::new(StringArray::from(std::mem::take(v))),
                    Buf::F32(v) => Arc::new(Float32Array::from(std::mem::take(v))),
                    Buf::F64(v) => Arc::new(Float64Array::from(std::mem::take(v))),
                    Buf::U32(v) => Arc::new(UInt32Array::from(std::mem::take(v))),
                    Buf::I32(v) => Arc::new(Int32Array::from(std::mem::take(v))),
                    Buf::Bool(v) => Arc::new(BooleanArray::from(std::mem::take(v))),
                }
            })
            .collect();
        let batch = RecordBatch::try_new(self.schema.clone(), arrays)?;
        self.w.write(&batch)?;
        self.total += self.rows;
        self.rows = 0;
        Ok(())
    }

    pub fn finish(mut self) -> Result<usize> {
        self.flush()?;
        self.w.close()?;
        Ok(self.total)
    }
}

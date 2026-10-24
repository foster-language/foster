//! One independently generated embedded Foster bytecode tool.
pub const BYTECODE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tool.fbc"));

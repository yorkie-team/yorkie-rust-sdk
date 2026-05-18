#![forbid(unsafe_code)]
//! Protocol types and converters for Yorkie.

pub mod converter;
pub mod resources;

/// The protobuf package used by Yorkie SDK RPCs.
pub const YORKIE_PROTO_PACKAGE: &str = "yorkie.v1";

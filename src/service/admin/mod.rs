//! Admin 服务

#![allow(dead_code)]

pub mod error;
pub mod service;

pub use error::AdminServiceError;
pub use service::AdminService;

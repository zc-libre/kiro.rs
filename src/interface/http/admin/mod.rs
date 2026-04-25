//! Admin API HTTP 接口

#![allow(dead_code)]

pub mod dto;
pub mod handlers;
pub mod middleware;
pub mod router;

pub use middleware::AdminState;
pub use router::create_admin_router;

//! Admin API HTTP 接口

pub mod dto;
pub mod handlers;
pub mod middleware;
pub mod router;

pub use middleware::AdminState;
pub use router::create_admin_router;

pub mod orm;
pub mod router;
pub mod settings;
pub mod template;
pub use cobalto_derive::Model;

inventory::collect!(crate::orm::Migration);

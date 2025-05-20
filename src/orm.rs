// cobalto/src/orm.rs

/// The core trait marking a struct as a Cobalto Model.
/// Can be derived or implemented for table mapping, migrations, etc.
pub trait Model: Sized + Send + Sync + 'static {
    fn table_name() -> &'static str;
}

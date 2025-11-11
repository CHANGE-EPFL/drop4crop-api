pub use sea_orm_migration::prelude::*;

mod m20250101_000001_consolidated_schema;
mod m20251111_142938_add_layer_statistics;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20250101_000001_consolidated_schema::Migration),
            Box::new(m20251111_142938_add_layer_statistics::Migration),
        ]
    }
}
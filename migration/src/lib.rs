pub use sea_orm_migration::prelude::*;

mod m20250101_000001_consolidated_schema;
mod m20251111_142938_add_layer_statistics;
mod m20251120_000001_enable_pg_trgm;
mod m20251126_000001_add_layer_total_views;
mod m20251202_000001_add_style_interpolation_type;
mod m20251203_000001_add_style_label_settings;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20250101_000001_consolidated_schema::Migration),
            Box::new(m20251111_142938_add_layer_statistics::Migration),
            Box::new(m20251120_000001_enable_pg_trgm::Migration),
            Box::new(m20251126_000001_add_layer_total_views::Migration),
            Box::new(m20251202_000001_add_style_interpolation_type::Migration),
            Box::new(m20251203_000001_add_style_label_settings::Migration),
        ]
    }
}
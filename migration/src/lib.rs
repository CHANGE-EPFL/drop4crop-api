pub use sea_orm_migration::prelude::*;

mod m20250101_000001_consolidated_schema;
mod m20251111_142938_add_layer_statistics;
mod m20251120_000001_enable_pg_trgm;
mod m20251126_000001_add_layer_total_views;
mod m20251202_000001_add_style_interpolation_type;
mod m20251203_000001_add_style_label_settings;
mod m20251203_000002_add_layer_stats_status;
mod m20251203_000003_add_stats_status_value;
mod m20260413_000001_add_project_table;
mod m20260413_000002_add_reference_tables;
mod m20260413_000003_add_showcase_item_table;
mod m20260413_000004_add_project_scoping;
mod m20260420_000001_add_time_axis_config;
mod m20260423_000001_add_project_relation_sort_order;

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
            Box::new(m20251203_000002_add_layer_stats_status::Migration),
            Box::new(m20251203_000003_add_stats_status_value::Migration),
            Box::new(m20260413_000001_add_project_table::Migration),
            Box::new(m20260413_000002_add_reference_tables::Migration),
            Box::new(m20260413_000003_add_showcase_item_table::Migration),
            Box::new(m20260413_000004_add_project_scoping::Migration),
            Box::new(m20260420_000001_add_time_axis_config::Migration),
            Box::new(m20260423_000001_add_project_relation_sort_order::Migration),
        ]
    }
}
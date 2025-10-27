use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Enable PostGIS extensions for PostgreSQL
        if manager.get_database_backend() == sea_orm::DatabaseBackend::Postgres {
            manager
                .get_connection()
                .execute_unprepared("CREATE EXTENSION IF NOT EXISTS \"uuid-ossp\";")
                .await?;
            manager
                .get_connection()
                .execute_unprepared("CREATE EXTENSION IF NOT EXISTS \"postgis\";")
                .await?;
        }

        // Enable fuzzystrmatch extension
        if manager.get_database_backend() == sea_orm::DatabaseBackend::Postgres {
            manager
                .get_connection()
                .execute_unprepared("CREATE EXTENSION IF NOT EXISTS \"fuzzystrmatch\";")
                .await?;
        }

        // Enable PostGIS topology extensions
        if manager.get_database_backend() == sea_orm::DatabaseBackend::Postgres {
            manager
                .get_connection()
                .execute_unprepared("CREATE EXTENSION IF NOT EXISTS \"postgis_topology\";")
                .await?;
        }

        // Create PostGIS schemas
        if manager.get_database_backend() == sea_orm::DatabaseBackend::Postgres {
            manager
                .get_connection()
                .execute_unprepared("CREATE SCHEMA IF NOT EXISTS tiger;")
                .await?;
            manager
                .get_connection()
                .execute_unprepared("CREATE SCHEMA IF NOT EXISTS tiger_data;")
                .await?;
            manager
                .get_connection()
                .execute_unprepared("CREATE SCHEMA IF NOT EXISTS topology;")
                .await?;
            manager
                .get_connection()
                .execute_unprepared("COMMENT ON SCHEMA topology IS 'PostGIS Topology schema';")
                .await?;
        }

        // Create alembic_version table for migration tracking
        manager
            .create_table(
                Table::create()
                    .table(AlembicVersion::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AlembicVersion::VersionNum)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .to_owned(),
            )
            .await?;

        // Create style table
        let mut style_table = Table::create()
            .table(Style::Table)
            .if_not_exists()
            .col(
                ColumnDef::new(Style::Name)
                    .string()
                    .not_null()
                    .unique_key(),
            )
            .col(
                ColumnDef::new(Style::LastUpdated)
                    .timestamp_with_time_zone()
                    .not_null()
                    .default(Expr::current_timestamp()),
            )
            .col(ColumnDef::new(Style::Style).json())
            .to_owned();

        // Add ID column with appropriate type and default based on database backend
        match manager.get_database_backend() {
            sea_orm::DatabaseBackend::Postgres => {
                style_table.col(
                    ColumnDef::new(Style::Id)
                        .uuid()
                        .not_null()
                        .unique_key(),
                );
            }
            sea_orm::DatabaseBackend::Sqlite => {
                style_table.col(
                    ColumnDef::new(Style::Id)
                        .uuid()
                        .not_null()
                        .unique_key(),
                );
            }
            _ => {
                return Err(DbErr::Custom("Unsupported database backend".to_string()));
            }
        }

        // Add iterator column for primary key
        style_table
            .col(
                ColumnDef::new(Style::Iterator)
                    .integer()
                    .not_null()
                    .auto_increment()
                    .primary_key(),
            )
            .index(
                Index::create()
                    .name("style_iterator_seq")
                    .col(Style::Iterator)
                    .unique(),
            );

        manager.create_table(style_table).await?;

        // Create country table with PostGIS geometry
        let mut country_table = Table::create()
            .table(Country::Table)
            .if_not_exists()
            .col(
                ColumnDef::new(Country::Name)
                    .string()
                    .not_null()
                    .unique_key(),
            )
            .col(
                ColumnDef::new(Country::IsoA2)
                    .string()
                    .not_null(),
            )
            .col(
                ColumnDef::new(Country::IsoA3)
                    .string()
                    .not_null(),
            )
            .col(
                ColumnDef::new(Country::IsoN3)
                    .integer()
                    .not_null(),
            )
            .col(ColumnDef::new(Country::Geom).custom("GEOMETRY(MULTIPOLYGON, 4326)"))
            .to_owned();

        // Add ID column with appropriate type and default based on database backend
        match manager.get_database_backend() {
            sea_orm::DatabaseBackend::Postgres => {
                country_table.col(
                    ColumnDef::new(Country::Id)
                        .uuid()
                        .not_null()
                        .unique_key(),
                );
            }
            sea_orm::DatabaseBackend::Sqlite => {
                country_table.col(
                    ColumnDef::new(Country::Id)
                        .uuid()
                        .not_null()
                        .unique_key(),
                );
            }
            _ => {
                return Err(DbErr::Custom("Unsupported database backend".to_string()));
            }
        }

        // Add iterator column for primary key
        country_table
            .col(
                ColumnDef::new(Country::Iterator)
                    .integer()
                    .not_null()
                    .auto_increment()
                    .primary_key(),
            )
            .index(
                Index::create()
                    .name("country_iterator_seq")
                    .col(Country::Iterator)
                    .unique(),
            );

        manager.create_table(country_table).await?;

        // Create layer table
        let mut layer_table = Table::create()
            .table(Layer::Table)
            .if_not_exists()
            .col(
                ColumnDef::new(Layer::LayerName)
                    .string()
                    .unique_key(),
            )
            .col(ColumnDef::new(Layer::Crop).string())
            .col(ColumnDef::new(Layer::WaterModel).string())
            .col(ColumnDef::new(Layer::ClimateModel).string())
            .col(ColumnDef::new(Layer::Scenario).string())
            .col(ColumnDef::new(Layer::Variable).string())
            .col(ColumnDef::new(Layer::Year).integer())
            .col(
                ColumnDef::new(Layer::LastUpdated)
                    .timestamp_with_time_zone()
                    .not_null()
                    .default(Expr::current_timestamp()),
            )
            .col(
                ColumnDef::new(Layer::UploadedAt)
                    .timestamp_with_time_zone()
                    .not_null()
                    .default(Expr::current_timestamp()),
            )
            .col(ColumnDef::new(Layer::GlobalAverage).double())
            .col(ColumnDef::new(Layer::Filename).string())
            .col(ColumnDef::new(Layer::MinValue).double())
            .col(ColumnDef::new(Layer::MaxValue).double())
            .col(ColumnDef::new(Layer::StyleId).uuid())
            .col(
                ColumnDef::new(Layer::IsCropSpecific)
                    .boolean()
                    .not_null()
                    .default(false),
            )
            .col(
                ColumnDef::new(Layer::Enabled)
                    .boolean()
                    .not_null()
                    .default(true),
            )
            .to_owned();

        // Add ID column with appropriate type and default based on database backend
        match manager.get_database_backend() {
            sea_orm::DatabaseBackend::Postgres => {
                layer_table.col(
                    ColumnDef::new(Layer::Id)
                        .uuid()
                        .not_null()
                        .unique_key(),
                );
            }
            sea_orm::DatabaseBackend::Sqlite => {
                layer_table.col(
                    ColumnDef::new(Layer::Id)
                        .uuid()
                        .not_null()
                        .unique_key(),
                );
            }
            _ => {
                return Err(DbErr::Custom("Unsupported database backend".to_string()));
            }
        }

        // Add iterator column for primary key
        layer_table
            .col(
                ColumnDef::new(Layer::Iterator)
                    .integer()
                    .not_null()
                    .auto_increment()
                    .primary_key(),
            )
            .index(
                Index::create()
                    .name("layer_iterator_seq")
                    .col(Layer::Iterator)
                    .unique(),
            );

        // Add unique constraint for layer identification
        layer_table
            .index(
                Index::create()
                    .name("layer_crop_year_variable_scenario_climate_model_water_model_key")
                    .col(Layer::Crop)
                    .col(Layer::Year)
                    .col(Layer::Variable)
                    .col(Layer::Scenario)
                    .col(Layer::ClimateModel)
                    .col(Layer::WaterModel)
                    .unique(),
            )
            .foreign_key(
                ForeignKey::create()
                    .name("layer_style_id_fkey")
                    .from(Layer::Table, Layer::StyleId)
                    .to(Style::Table, Style::Id)
                    .on_delete(ForeignKeyAction::SetNull),
            );

        manager.create_table(layer_table).await?;

        // Create layercountrylink table (junction table)
        manager
            .create_table(
                Table::create()
                    .table(LayerCountryLink::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(LayerCountryLink::CountryId).uuid().not_null())
                    .col(ColumnDef::new(LayerCountryLink::LayerId).uuid().not_null())
                    .col(ColumnDef::new(LayerCountryLink::VarWf).double())
                    .col(ColumnDef::new(LayerCountryLink::VarWfb).double())
                    .col(ColumnDef::new(LayerCountryLink::VarWfg).double())
                    .col(ColumnDef::new(LayerCountryLink::VarVwc).double())
                    .col(ColumnDef::new(LayerCountryLink::VarVwcb).double())
                    .col(ColumnDef::new(LayerCountryLink::VarVwcg).double())
                    .col(ColumnDef::new(LayerCountryLink::VarVdb).double())
                    .col(ColumnDef::new(LayerCountryLink::VarWdg).double())
                    .primary_key(
                        Index::create()
                            .col(LayerCountryLink::CountryId)
                            .col(LayerCountryLink::LayerId),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("layercountrylink_country_id_fkey")
                            .from(LayerCountryLink::Table, LayerCountryLink::CountryId)
                            .to(Country::Table, Country::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("layercountrylink_layer_id_fkey")
                            .from(LayerCountryLink::Table, LayerCountryLink::LayerId)
                            .to(Layer::Table, Layer::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Create indexes for country table
        manager
            .create_index(
                Index::create()
                    .name("ix_country_name")
                    .table(Country::Table)
                    .col(Country::Name)
                    .to_owned(),
            )
            .await?;

        // Create spatial index for country geometry (PostgreSQL only)
        if manager.get_database_backend() == sea_orm::DatabaseBackend::Postgres {
            manager
                .get_connection()
                .execute_unprepared("CREATE INDEX idx_country_geom ON public.country USING gist (geom);")
                .await?;
        }

        // Create indexes for layer table
        let layer_indexes = vec![
            ("ix_layer_climate_model", Layer::ClimateModel),
            ("ix_layer_crop", Layer::Crop),
            ("ix_layer_id", Layer::Id),
            ("ix_layer_layer_name", Layer::LayerName),
            ("ix_layer_scenario", Layer::Scenario),
            ("ix_layer_variable", Layer::Variable),
            ("ix_layer_water_model", Layer::WaterModel),
            ("ix_layer_year", Layer::Year),
            ("ix_layer_global_average", Layer::GlobalAverage),
        ];

        for (index_name, column) in layer_indexes {
            manager
                .create_index(
                    Index::create()
                        .name(index_name)
                        .table(Layer::Table)
                        .col(column)
                        .to_owned(),
                )
                .await?;
        }

        // Create indexes for style table
        let style_indexes = vec![
            ("ix_style_name", Style::Name),
            ("ix_style_id", Style::Id),
            ("ix_style_iterator", Style::Iterator),
        ];

        for (index_name, column) in style_indexes {
            manager
                .create_index(
                    Index::create()
                        .name(index_name)
                        .table(Style::Table)
                        .col(column)
                        .to_owned(),
                )
                .await?;
        }

        // Insert country data from GeoJSON if resource file exists
        if manager.get_database_backend() == sea_orm::DatabaseBackend::Postgres {
            // Try to insert country data if GeoJSON file exists
            if std::path::Path::new("migrations/resources/ne_50m_admin_0_countries.geojson").exists() {
                // Note: This data insertion would typically be handled by a separate seed script
                // For now, we're just ensuring that schema is properly created
                println!("Country GeoJSON file found. Consider running a seed script to populate data.");
            }
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop tables in reverse dependency order
        manager
            .drop_table(Table::drop().table(LayerCountryLink::Table).if_exists().to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Layer::Table).if_exists().to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Country::Table).if_exists().to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Style::Table).if_exists().to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(AlembicVersion::Table).if_exists().to_owned())
            .await?;

        // Drop PostGIS schemas (PostgreSQL only)
        if manager.get_database_backend() == sea_orm::DatabaseBackend::Postgres {
            manager
                .get_connection()
                .execute_unprepared("DROP SCHEMA IF EXISTS topology CASCADE;")
                .await
                .ok();
            manager
                .get_connection()
                .execute_unprepared("DROP SCHEMA IF EXISTS tiger_data CASCADE;")
                .await
                .ok();
            manager
                .get_connection()
                .execute_unprepared("DROP SCHEMA IF EXISTS tiger CASCADE;")
                .await
                .ok();
        }

        Ok(())
    }
}

// Table and column identifiers
#[derive(DeriveIden)]
pub enum AlembicVersion {
    Table,
    VersionNum,
}

#[derive(DeriveIden)]
pub enum Style {
    Table,
    Id,
    Name,
    LastUpdated,
    Iterator,
    Style,
}

#[derive(DeriveIden)]
pub enum Country {
    Table,
    Id,
    Name,
    IsoA2,
    IsoA3,
    IsoN3,
    Geom,
    Iterator,
}

#[derive(DeriveIden)]
pub enum Layer {
    Table,
    Id,
    LayerName,
    Crop,
    WaterModel,
    ClimateModel,
    Scenario,
    Variable,
    Year,
    LastUpdated,
    UploadedAt,
    GlobalAverage,
    Filename,
    MinValue,
    MaxValue,
    StyleId,
    IsCropSpecific,
    Enabled,
    Iterator,
}

#[derive(DeriveIden)]
pub enum LayerCountryLink {
    Table,
    CountryId,
    LayerId,
    VarWf,
    VarWfb,
    VarWfg,
    VarVwc,
    VarVwcb,
    VarVwcg,
    VarVdb,
    VarWdg,
}


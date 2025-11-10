use sea_orm_migration::prelude::*;
use serde_json::Value;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Check if alembic_version table exists (indicating migration from Alembic)
        // If it exists, we skip all schema creation and just clean up the alembic table
        let db = manager.get_connection();

        let alembic_exists = if manager.get_database_backend() == sea_orm::DatabaseBackend::Postgres {
            let result = db.query_one(sea_orm::Statement::from_string(
                manager.get_database_backend(),
                "SELECT EXISTS (SELECT FROM information_schema.tables WHERE table_schema = 'public' AND table_name = 'alembic_version') as table_exists".to_string()
            )).await;

            match result {
                Ok(Some(row)) => row.try_get::<bool>("", "table_exists").unwrap_or(false),
                _ => false,
            }
        } else {
            // For SQLite or other databases
            let result = db.query_one(sea_orm::Statement::from_string(
                manager.get_database_backend(),
                "SELECT name FROM sqlite_master WHERE type='table' AND name='alembic_version'".to_string()
            )).await;

            result.is_ok() && result.unwrap().is_some()
        };

        if alembic_exists {
            println!("Alembic version table detected. Skipping schema creation and removing alembic_version table...");

            // Drop the alembic_version table to complete migration to Sea-ORM
            manager
                .drop_table(
                    Table::drop()
                        .table(AlembicVersion::Table)
                        .if_exists()
                        .to_owned(),
                )
                .await?;

            println!("Successfully migrated from Alembic to Sea-ORM migrations.");
            return Ok(());
        }

        println!("No Alembic version table found. Running full schema migration...");

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

        // Create style table
        let mut style_table = Table::create()
            .table(Style::Table)
            .if_not_exists()
            .col(ColumnDef::new(Style::Name).string().not_null().unique_key())
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
                style_table.col(ColumnDef::new(Style::Id).uuid().not_null().unique_key());
            }
            sea_orm::DatabaseBackend::Sqlite => {
                style_table.col(ColumnDef::new(Style::Id).uuid().not_null().unique_key());
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
            .col(ColumnDef::new(Country::IsoA2).string().not_null())
            .col(ColumnDef::new(Country::IsoA3).string().not_null())
            .col(ColumnDef::new(Country::IsoN3).integer().not_null())
            .col(ColumnDef::new(Country::Geom).custom("GEOMETRY(MULTIPOLYGON, 4326)"))
            .to_owned();

        // Add ID column with appropriate type and default based on database backend
        match manager.get_database_backend() {
            sea_orm::DatabaseBackend::Postgres => {
                country_table.col(ColumnDef::new(Country::Id).uuid().not_null().unique_key());
            }
            sea_orm::DatabaseBackend::Sqlite => {
                country_table.col(ColumnDef::new(Country::Id).uuid().not_null().unique_key());
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
            );

        manager.create_table(country_table).await?;

        // Create layer table
        let mut layer_table = Table::create()
            .table(Layer::Table)
            .if_not_exists()
            .col(ColumnDef::new(Layer::LayerName).string().unique_key())
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
                layer_table.col(ColumnDef::new(Layer::Id).uuid().not_null().unique_key());
            }
            sea_orm::DatabaseBackend::Sqlite => {
                layer_table.col(ColumnDef::new(Layer::Id).uuid().not_null().unique_key());
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
                    .col(
                        ColumnDef::new(LayerCountryLink::CountryId)
                            .uuid()
                            .not_null(),
                    )
                    .col(ColumnDef::new(LayerCountryLink::LayerId).uuid().not_null())
                    .col(ColumnDef::new(LayerCountryLink::VarWf).double())
                    .col(ColumnDef::new(LayerCountryLink::VarWfb).double())
                    .col(ColumnDef::new(LayerCountryLink::VarWfg).double())
                    .col(ColumnDef::new(LayerCountryLink::VarVwc).double())
                    .col(ColumnDef::new(LayerCountryLink::VarVwcb).double())
                    .col(ColumnDef::new(LayerCountryLink::VarVwcg).double())
                    .col(ColumnDef::new(LayerCountryLink::VarWdg).double())
                    .col(ColumnDef::new(LayerCountryLink::VarWdb).double())
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
                .execute_unprepared(
                    "CREATE INDEX idx_country_geom ON public.country USING gist (geom);",
                )
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
            // Try to load and insert country data from GeoJSON file
            let geojson_path =
                std::path::Path::new("migration/resources/ne_50m_admin_0_countries.geojson");
            if geojson_path.exists() {
                match std::fs::read_to_string(geojson_path) {
                    Ok(json_content) => {
                        match serde_json::from_str::<Value>(&json_content) {
                            Ok(geojson_data) => {
                                if let Some(features) =
                                    geojson_data.get("features").and_then(|f| f.as_array())
                                {
                                    let mut country_count = 0;
                                    for feature in features {
                                        if let (Some(properties), Some(geometry)) =
                                            (feature.get("properties"), feature.get("geometry"))
                                        {
                                            if let (
                                                Some(name),
                                                Some(iso_a2),
                                                Some(iso_a3),
                                                Some(iso_n3),
                                            ) = (
                                                properties.get("NAME").and_then(|n| n.as_str()),
                                                properties.get("ISO_A2").and_then(|n| n.as_str()),
                                                properties.get("ISO_A3").and_then(|n| n.as_str()),
                                                properties.get("ISO_N3").and_then(|n| n.as_str()),
                                            ) {
                                                // Only insert countries with valid ISO codes
                                                if !iso_a2.is_empty()
                                                    && !iso_a3.is_empty()
                                                    && !iso_n3.is_empty()
                                                {
                                                    if let Ok(geom_json) =
                                                        serde_json::to_string(geometry)
                                                    {
                                                        let sql = format!(
                                                            "INSERT INTO country (id, name, iso_a2, iso_a3, iso_n3, geom) VALUES (uuid_generate_v4(), '{}', '{}', '{}', {}, ST_SetSRID(ST_GeomFromGeoJSON('{}'), 4326))",
                                                            name.replace('\'', "''"), // Escape single quotes
                                                            iso_a2,
                                                            iso_a3,
                                                            iso_n3,
                                                            geom_json.replace('\'', "''") // Escape single quotes in JSON
                                                        );

                                                        match manager
                                                            .get_connection()
                                                            .execute_unprepared(&sql)
                                                            .await
                                                        {
                                                            Ok(_) => country_count += 1,
                                                            Err(e) => {
                                                                println!("Warning: Failed to insert country {}: {:?}", name, e);
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    println!(
                                        "Successfully loaded {} countries from GeoJSON",
                                        country_count
                                    );
                                } else {
                                    println!("No features found in GeoJSON file");
                                }
                            }
                            Err(_) => {
                                println!("Failed to parse GeoJSON: invalid format");
                            }
                        }
                    }
                    Err(e) => {
                        println!("Failed to read GeoJSON file: {:?}", e);
                    }
                }
            } else {
                println!("GeoJSON file not found at migration/resources/ne_50m_admin_0_countries.geojson");
            }
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop tables in reverse dependency order
        manager
            .drop_table(
                Table::drop()
                    .table(LayerCountryLink::Table)
                    .if_exists()
                    .to_owned(),
            )
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
            .drop_table(
                Table::drop()
                    .table(AlembicVersion::Table)
                    .if_exists()
                    .to_owned(),
            )
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
}

#[derive(DeriveIden)]
pub enum Style {
    Table,
    Id,
    Name,
    LastUpdated,
    Iterator,
    #[allow(clippy::enum_variant_names)]
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
    #[allow(clippy::enum_variant_names)]
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
    #[sea_orm(iden = "layercountrylink")]
    Table,
    CountryId,
    LayerId,
    VarWf,
    VarWfb,
    VarWfg,
    VarVwc,
    VarVwcb,
    VarVwcg,
    VarWdg,
    VarWdb,
}

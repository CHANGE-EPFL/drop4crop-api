use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // variable.has_time — declares whether the variable varies over time.
        // Default true; backfilled to !is_crop_specific so existing crop-specific
        // rows stay correct.
        manager
            .alter_table(
                Table::alter()
                    .table(Variable::Table)
                    .add_column(
                        ColumnDef::new(Variable::HasTime)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .to_owned(),
            )
            .await?;

        db.execute_unprepared("UPDATE variable SET has_time = NOT is_crop_specific")
            .await?;

        // project.year_axis — JSON describing the timeline for the public UI.
        // Nullable: a project with no time-varying layers can leave this unset.
        // Shape: {"mode":"range","min":2000,"max":2090,"step":10}
        //    or: {"mode":"list","values":[2020,2050,2090]}
        manager
            .alter_table(
                Table::alter()
                    .table(Project::Table)
                    .add_column(ColumnDef::new(Project::YearAxis).json_binary())
                    .to_owned(),
            )
            .await?;

        // Seed the existing active project with its decadal ISIMIP axis.
        db.execute_unprepared(
            "UPDATE project \
             SET year_axis = '{\"mode\":\"range\",\"min\":2000,\"max\":2090,\"step\":10}'::jsonb \
             WHERE slug = 'crop-water-use'",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Project::Table)
                    .drop_column(Project::YearAxis)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Variable::Table)
                    .drop_column(Variable::HasTime)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum Variable {
    Table,
    HasTime,
}

#[derive(DeriveIden)]
enum Project {
    Table,
    YearAxis,
}

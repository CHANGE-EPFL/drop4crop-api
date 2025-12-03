use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Add interpolation_type column to style table
        // Values: 'linear' (default, smooth interpolation) or 'discrete' (stepped/bucketed)
        manager
            .alter_table(
                Table::alter()
                    .table(Style::Table)
                    .add_column(
                        ColumnDef::new(Style::InterpolationType)
                            .string()
                            .not_null()
                            .default("linear"),
                    )
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Style::Table)
                    .drop_column(Style::InterpolationType)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum Style {
    Table,
    InterpolationType,
}

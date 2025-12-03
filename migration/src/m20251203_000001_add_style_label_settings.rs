use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Add label_display_mode column: 'auto' (default) or 'manual'
        // - auto: Show a limited number of evenly-spaced labels (default 5)
        // - manual: Show all labels from the style definition
        manager
            .alter_table(
                Table::alter()
                    .table(Style::Table)
                    .add_column(
                        ColumnDef::new(Style::LabelDisplayMode)
                            .string()
                            .not_null()
                            .default("auto"),
                    )
                    .to_owned(),
            )
            .await?;

        // Add label_count column: Optional override for number of labels to display
        // Only used when label_display_mode is 'auto'
        // NULL means use default (5 labels)
        manager
            .alter_table(
                Table::alter()
                    .table(Style::Table)
                    .add_column(
                        ColumnDef::new(Style::LabelCount)
                            .integer()
                            .null(),
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
                    .drop_column(Style::LabelDisplayMode)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Style::Table)
                    .drop_column(Style::LabelCount)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum Style {
    Table,
    LabelDisplayMode,
    LabelCount,
}

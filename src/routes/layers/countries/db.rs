use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "layercountrylink")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub country_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub layer_id: Uuid,
    #[sea_orm(column_type = "Double", nullable)]
    pub var_wf: Option<f64>,
    #[sea_orm(column_type = "Double", nullable)]
    pub var_wfb: Option<f64>,
    #[sea_orm(column_type = "Double", nullable)]
    pub var_wfg: Option<f64>,
    #[sea_orm(column_type = "Double", nullable)]
    pub var_vwc: Option<f64>,
    #[sea_orm(column_type = "Double", nullable)]
    pub var_vwcb: Option<f64>,
    #[sea_orm(column_type = "Double", nullable)]
    pub var_vwcg: Option<f64>,
    #[sea_orm(column_type = "Double", nullable)]
    pub var_wdb: Option<f64>,
    #[sea_orm(column_type = "Double", nullable)]
    pub var_wdg: Option<f64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::country::Entity",
        from = "Column::CountryId",
        to = "super::country::Column::Id",
        on_update = "NoAction",
        on_delete = "NoAction"
    )]
    Country,
    #[sea_orm(
        belongs_to = "super::layer::Entity",
        from = "Column::LayerId",
        to = "super::layer::Column::Id",
        on_update = "NoAction",
        on_delete = "NoAction"
    )]
    Layer,
}

impl Related<super::country::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Country.def()
    }
}

impl Related<super::layer::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Layer.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

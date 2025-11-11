use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

pub mod layer_statistics {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
    #[sea_orm(table_name = "layer_statistics")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub layer_id: Uuid,
        pub stat_date: chrono::NaiveDate,
        pub last_accessed_at: chrono::DateTime<chrono::Utc>,
        pub xyz_tile_count: i32,
        pub cog_download_count: i32,
        pub pixel_query_count: i32,
        pub stac_request_count: i32,
        pub other_request_count: i32,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(
            belongs_to = "crate::routes::layers::db::Entity",
            from = "Column::LayerId",
            to = "crate::routes::layers::db::Column::Id"
        )]
        Layer,
    }

    impl Related<crate::routes::layers::db::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Layer.def()
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use chrono::NaiveDate;
use sea_orm::{
    ColumnTrait, EntityTrait, FromQueryResult, QueryFilter, QueryOrder, QuerySelect, RelationTrait,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::common::state::AppState;
use crate::routes::admin::db::layer_statistics;
use crate::routes::layers::db as layer;

#[derive(Deserialize)]
pub(super) struct ActivityQuery {
    start_date: Option<String>,
    end_date: Option<String>,
}

#[derive(FromQueryResult)]
struct ActivityBounds {
    earliest_date: Option<NaiveDate>,
    latest_date: Option<NaiveDate>,
}

#[derive(FromQueryResult)]
struct ActivityLayerIdentity {
    layer_id: uuid::Uuid,
    layer_name: Option<String>,
}

#[derive(FromQueryResult)]
struct ActivityRow {
    layer_id: uuid::Uuid,
    stat_date: NaiveDate,
    total_requests: i64,
}

#[derive(Serialize)]
struct ActivityLayer {
    layer_id: String,
    layer_name: String,
    daily_requests: Vec<i64>,
}

#[derive(Serialize)]
pub(super) struct ActivityResponse {
    earliest_date: Option<String>,
    latest_date: Option<String>,
    start_date: Option<String>,
    end_date: Option<String>,
    dates: Vec<String>,
    layers: Vec<ActivityLayer>,
}

fn parse_date(value: Option<&str>) -> Result<Option<NaiveDate>, StatusCode> {
    value
        .map(|date| {
            NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|_| StatusCode::BAD_REQUEST)
        })
        .transpose()
}

fn total_requests_expr() -> sea_orm::sea_query::SimpleExpr {
    use sea_orm::sea_query::Expr;
    Expr::col(layer_statistics::Column::XyzTileCount)
        .sum()
        .add(Expr::col(layer_statistics::Column::CogDownloadCount).sum())
        .add(Expr::col(layer_statistics::Column::PixelQueryCount).sum())
        .add(Expr::col(layer_statistics::Column::StacRequestCount).sum())
}

pub(super) async fn get_activity(
    State(app_state): State<AppState>,
    Query(params): Query<ActivityQuery>,
) -> Result<Json<ActivityResponse>, StatusCode> {
    let db = &app_state.db;
    let requested_start = parse_date(params.start_date.as_deref())?;
    let requested_end = parse_date(params.end_date.as_deref())?;

    let bounds = layer_statistics::Entity::find()
        .select_only()
        .column_as(
            sea_orm::sea_query::Expr::col(layer_statistics::Column::StatDate).min(),
            "earliest_date",
        )
        .column_as(
            sea_orm::sea_query::Expr::col(layer_statistics::Column::StatDate).max(),
            "latest_date",
        )
        .into_model::<ActivityBounds>()
        .one(db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .unwrap_or(ActivityBounds {
            earliest_date: None,
            latest_date: None,
        });

    let start = requested_start.or(bounds.earliest_date);
    let end = requested_end.or(bounds.latest_date);
    if matches!((start, end), (Some(start), Some(end)) if start > end) {
        return Err(StatusCode::BAD_REQUEST);
    }

    let dates: Vec<NaiveDate> = match (start, end) {
        (Some(start), Some(end)) => start.iter_days().take_while(|day| *day <= end).collect(),
        _ => Vec::new(),
    };
    let date_indexes: HashMap<NaiveDate, usize> = dates
        .iter()
        .enumerate()
        .map(|(index, date)| (*date, index))
        .collect();

    let identities = layer_statistics::Entity::find()
        .select_only()
        .column(layer_statistics::Column::LayerId)
        .column(layer::Column::LayerName)
        .join(
            sea_orm::JoinType::InnerJoin,
            layer_statistics::Relation::Layer.def(),
        )
        .distinct()
        .order_by_asc(layer::Column::LayerName)
        .order_by_asc(layer_statistics::Column::LayerId)
        .into_model::<ActivityLayerIdentity>()
        .all(db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut rows_query = layer_statistics::Entity::find();
    if let Some(start) = start {
        rows_query = rows_query.filter(layer_statistics::Column::StatDate.gte(start));
    }
    if let Some(end) = end {
        rows_query = rows_query.filter(layer_statistics::Column::StatDate.lte(end));
    }
    let rows = rows_query
        .select_only()
        .column(layer_statistics::Column::LayerId)
        .column(layer_statistics::Column::StatDate)
        .column_as(total_requests_expr(), "total_requests")
        .group_by(layer_statistics::Column::LayerId)
        .group_by(layer_statistics::Column::StatDate)
        .order_by_asc(layer_statistics::Column::LayerId)
        .order_by_asc(layer_statistics::Column::StatDate)
        .into_model::<ActivityRow>()
        .all(db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut layers: Vec<ActivityLayer> = identities
        .into_iter()
        .map(|identity| ActivityLayer {
            layer_id: identity.layer_id.to_string(),
            layer_name: identity
                .layer_name
                .unwrap_or_else(|| identity.layer_id.to_string()),
            daily_requests: vec![0; dates.len()],
        })
        .collect();
    let layer_indexes: HashMap<String, usize> = layers
        .iter()
        .enumerate()
        .map(|(index, layer)| (layer.layer_id.clone(), index))
        .collect();
    for row in rows {
        let Some(layer_index) = layer_indexes.get(&row.layer_id.to_string()) else {
            continue;
        };
        let Some(date_index) = date_indexes.get(&row.stat_date) else {
            continue;
        };
        layers[*layer_index].daily_requests[*date_index] = row.total_requests;
    }

    Ok(Json(ActivityResponse {
        earliest_date: bounds.earliest_date.map(|date| date.to_string()),
        latest_date: bounds.latest_date.map(|date| date.to_string()),
        start_date: start.map(|date| date.to_string()),
        end_date: end.map(|date| date.to_string()),
        dates: dates.into_iter().map(|date| date.to_string()).collect(),
        layers,
    }))
}

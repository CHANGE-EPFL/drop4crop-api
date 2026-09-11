use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use chrono::NaiveDate;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, FromQueryResult, JoinType, QueryFilter,
    QueryOrder, QuerySelect, RelationTrait,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::common::state::AppState;
use crate::routes::admin::db::layer_statistics;
use crate::routes::climate_models::db as climate_model;
use crate::routes::crops::db as crop;
use crate::routes::layers::db as layer;
use crate::routes::projects::db as project;
use crate::routes::scenarios::db as scenario;
use crate::routes::variables::db as variable;
use crate::routes::water_models::db as water_model;

const DEFAULT_LIMIT: u64 = 25;
const MAX_LIMIT: u64 = 500;

#[derive(Deserialize)]
pub(super) struct ActivityQuery {
    start_date: Option<String>,
    end_date: Option<String>,
    limit: Option<u64>,
    offset: Option<u64>,
}

#[derive(FromQueryResult)]
struct ActivityBounds {
    earliest_date: Option<NaiveDate>,
    latest_date: Option<NaiveDate>,
}

#[derive(FromQueryResult)]
struct DailyTotal {
    stat_date: NaiveDate,
    total_requests: i64,
}

#[derive(FromQueryResult)]
struct LayerTotal {
    layer_id: uuid::Uuid,
    total_requests: i64,
}

#[derive(FromQueryResult)]
struct LayerCount {
    total_layers: i64,
}

#[derive(FromQueryResult)]
struct ActivityLayerIdentity {
    id: uuid::Uuid,
    layer_name: Option<String>,
    project: Option<String>,
    crop: Option<String>,
    water_model: Option<String>,
    climate_model: Option<String>,
    scenario: Option<String>,
    variable: Option<String>,
    year: Option<i32>,
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
    project: Option<String>,
    crop: Option<String>,
    water_model: Option<String>,
    climate_model: Option<String>,
    scenario: Option<String>,
    variable: Option<String>,
    year: Option<i32>,
    total_requests: i64,
    daily_requests: Vec<i64>,
}

/// Layers active in the period, most requested first, one page at a time. `daily_totals`
/// covers every day between `earliest_date` and `latest_date` summed over all layers.
#[derive(Serialize)]
pub(super) struct ActivityResponse {
    earliest_date: Option<String>,
    latest_date: Option<String>,
    start_date: Option<String>,
    end_date: Option<String>,
    daily_totals: Vec<i64>,
    dates: Vec<String>,
    layers: Vec<ActivityLayer>,
    total_layers: u64,
    offset: u64,
    limit: u64,
}

fn parse_date(value: Option<&str>) -> Result<Option<NaiveDate>, StatusCode> {
    value
        .map(|date| {
            NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|_| StatusCode::BAD_REQUEST)
        })
        .transpose()
}

fn total_requests_expr() -> sea_orm::sea_query::SimpleExpr {
    Expr::col(layer_statistics::Column::XyzTileCount)
        .sum()
        .add(Expr::col(layer_statistics::Column::CogDownloadCount).sum())
        .add(Expr::col(layer_statistics::Column::PixelQueryCount).sum())
        .add(Expr::col(layer_statistics::Column::StacRequestCount).sum())
}

fn days_between(start: Option<NaiveDate>, end: Option<NaiveDate>) -> Vec<NaiveDate> {
    match (start, end) {
        (Some(start), Some(end)) => start.iter_days().take_while(|day| *day <= end).collect(),
        _ => Vec::new(),
    }
}

fn date_indexes(dates: &[NaiveDate]) -> HashMap<NaiveDate, usize> {
    dates
        .iter()
        .enumerate()
        .map(|(index, date)| (*date, index))
        .collect()
}

fn period_rows(
    start: Option<NaiveDate>,
    end: Option<NaiveDate>,
) -> sea_orm::Select<layer_statistics::Entity> {
    let mut query = layer_statistics::Entity::find();
    if let Some(start) = start {
        query = query.filter(layer_statistics::Column::StatDate.gte(start));
    }
    if let Some(end) = end {
        query = query.filter(layer_statistics::Column::StatDate.lte(end));
    }
    query
}

async fn load_bounds(db: &DatabaseConnection) -> Result<ActivityBounds, StatusCode> {
    Ok(layer_statistics::Entity::find()
        .select_only()
        .column_as(
            Expr::col(layer_statistics::Column::StatDate).min(),
            "earliest_date",
        )
        .column_as(
            Expr::col(layer_statistics::Column::StatDate).max(),
            "latest_date",
        )
        .into_model::<ActivityBounds>()
        .one(db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .unwrap_or(ActivityBounds {
            earliest_date: None,
            latest_date: None,
        }))
}

async fn load_daily_totals(
    db: &DatabaseConnection,
    days: &[NaiveDate],
) -> Result<Vec<i64>, StatusCode> {
    let totals = layer_statistics::Entity::find()
        .select_only()
        .column(layer_statistics::Column::StatDate)
        .column_as(total_requests_expr(), "total_requests")
        .group_by(layer_statistics::Column::StatDate)
        .into_model::<DailyTotal>()
        .all(db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let indexes = date_indexes(days);
    let mut series = vec![0; days.len()];
    for total in totals {
        if let Some(index) = indexes.get(&total.stat_date) {
            series[*index] = total.total_requests;
        }
    }
    Ok(series)
}

async fn load_identities(
    db: &DatabaseConnection,
    ids: &[uuid::Uuid],
) -> Result<HashMap<uuid::Uuid, ActivityLayerIdentity>, StatusCode> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let identities = layer::Entity::find()
        .filter(layer::Column::Id.is_in(ids.iter().copied()))
        .select_only()
        .column(layer::Column::Id)
        .column(layer::Column::LayerName)
        .column(layer::Column::Year)
        .column_as(
            Expr::col((project::Entity, project::Column::Title)),
            "project",
        )
        .column_as(Expr::col((crop::Entity, crop::Column::Name)), "crop")
        .column_as(
            Expr::col((water_model::Entity, water_model::Column::Name)),
            "water_model",
        )
        .column_as(
            Expr::col((climate_model::Entity, climate_model::Column::Name)),
            "climate_model",
        )
        .column_as(
            Expr::col((scenario::Entity, scenario::Column::Name)),
            "scenario",
        )
        .column_as(
            Expr::col((variable::Entity, variable::Column::Name)),
            "variable",
        )
        .join(JoinType::LeftJoin, layer::Relation::Project.def())
        .join(JoinType::LeftJoin, layer::Relation::Crop.def())
        .join(JoinType::LeftJoin, layer::Relation::WaterModel.def())
        .join(JoinType::LeftJoin, layer::Relation::ClimateModel.def())
        .join(JoinType::LeftJoin, layer::Relation::Scenario.def())
        .join(JoinType::LeftJoin, layer::Relation::Variable.def())
        .into_model::<ActivityLayerIdentity>()
        .all(db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(identities
        .into_iter()
        .map(|identity| (identity.id, identity))
        .collect())
}

pub(super) async fn get_activity(
    State(app_state): State<AppState>,
    Query(params): Query<ActivityQuery>,
) -> Result<Json<ActivityResponse>, StatusCode> {
    let db = &app_state.db;
    let requested_start = parse_date(params.start_date.as_deref())?;
    let requested_end = parse_date(params.end_date.as_deref())?;
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let offset = params.offset.unwrap_or(0);

    let bounds = load_bounds(db).await?;
    let start = requested_start.or(bounds.earliest_date);
    let end = requested_end.or(bounds.latest_date);
    if matches!((start, end), (Some(start), Some(end)) if start > end) {
        return Err(StatusCode::BAD_REQUEST);
    }

    let all_days = days_between(bounds.earliest_date, bounds.latest_date);
    let daily_totals = load_daily_totals(db, &all_days).await?;

    let dates = days_between(start, end);
    let date_indexes = date_indexes(&dates);

    let total_layers = period_rows(start, end)
        .select_only()
        .column_as(
            Expr::col(layer_statistics::Column::LayerId).count_distinct(),
            "total_layers",
        )
        .into_model::<LayerCount>()
        .one(db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map_or(0, |count| count.total_layers);

    let page = period_rows(start, end)
        .select_only()
        .column(layer_statistics::Column::LayerId)
        .column_as(total_requests_expr(), "total_requests")
        .group_by(layer_statistics::Column::LayerId)
        .order_by_desc(Expr::cust("total_requests"))
        .order_by_asc(layer_statistics::Column::LayerId)
        .limit(limit)
        .offset(offset)
        .into_model::<LayerTotal>()
        .all(db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let ids: Vec<uuid::Uuid> = page.iter().map(|total| total.layer_id).collect();

    let rows = if ids.is_empty() {
        Vec::new()
    } else {
        period_rows(start, end)
            .filter(layer_statistics::Column::LayerId.is_in(ids.iter().copied()))
            .select_only()
            .column(layer_statistics::Column::LayerId)
            .column(layer_statistics::Column::StatDate)
            .column_as(total_requests_expr(), "total_requests")
            .group_by(layer_statistics::Column::LayerId)
            .group_by(layer_statistics::Column::StatDate)
            .into_model::<ActivityRow>()
            .all(db)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    };
    let mut identities = load_identities(db, &ids).await?;

    let mut layers: Vec<ActivityLayer> = page
        .into_iter()
        .map(|total| {
            let identity = identities.remove(&total.layer_id);
            ActivityLayer {
                layer_id: total.layer_id.to_string(),
                layer_name: identity
                    .as_ref()
                    .and_then(|identity| identity.layer_name.clone())
                    .unwrap_or_else(|| total.layer_id.to_string()),
                project: identity
                    .as_ref()
                    .and_then(|identity| identity.project.clone()),
                crop: identity.as_ref().and_then(|identity| identity.crop.clone()),
                water_model: identity
                    .as_ref()
                    .and_then(|identity| identity.water_model.clone()),
                climate_model: identity
                    .as_ref()
                    .and_then(|identity| identity.climate_model.clone()),
                scenario: identity
                    .as_ref()
                    .and_then(|identity| identity.scenario.clone()),
                variable: identity
                    .as_ref()
                    .and_then(|identity| identity.variable.clone()),
                year: identity.as_ref().and_then(|identity| identity.year),
                total_requests: total.total_requests,
                daily_requests: vec![0; dates.len()],
            }
        })
        .collect();
    let layer_indexes: HashMap<uuid::Uuid, usize> = ids
        .iter()
        .enumerate()
        .map(|(index, id)| (*id, index))
        .collect();
    for row in rows {
        let Some(layer_index) = layer_indexes.get(&row.layer_id) else {
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
        daily_totals,
        dates: dates.into_iter().map(|date| date.to_string()).collect(),
        layers,
        total_layers: u64::try_from(total_layers).unwrap_or(0),
        offset,
        limit,
    }))
}

from app.layers.models import (
    Layer,
    LayerCreate,
    LayerRead,
    LayerUpdate,
    LayerReadAuthenticated,
    LayerVariables,
    LayerGroupsRead,
    LayerUpdateBatch,
)
from app.db import get_session, AsyncSession
from fastapi import (
    Depends,
    HTTPException,
    APIRouter,
    Query,
    BackgroundTasks,
)
from uuid import UUID
from app.crud import CRUD
from app.layers.services import (
    get_count,
    get_all_authenticated,
    get_one,
    create_one,
    update_one,
)
from typing import Any
from sqlmodel import select
import httpx
from app.config import config
from app.auth import require_admin, get_user_info, User
from app.layers.geoserver import update_local_db_with_layer_style

router = APIRouter()


@router.post("/sync_styles", response_model=bool)
async def sync_style_from_geoserver_to_all_layers(
    background_tasks: BackgroundTasks,
    session: AsyncSession = Depends(get_session),
    user: User = Depends(require_admin),
) -> bool:
    """Syncs the style from geoserver to all layers"""

    # Get all layers
    query = select(
        Layer,
        Layer.id,
    )
    res = await session.exec(query)
    layers = res.all()

    for layer in layers:
        # Get the style from geoserver
        background_tasks.add_task(
            update_local_db_with_layer_style,
            layer_id=layer.id,
            session=session,
        )

    return True


@router.get("/groups", response_model=LayerGroupsRead)
async def get_groups(
    session: AsyncSession = Depends(get_session),
) -> LayerGroupsRead:
    """Get all unique groups
    This endpoint allows the menu to be populated with available keys
    """

    groups = {}
    for group in LayerVariables:
        # Get distinct values for each group

        column = getattr(Layer, group.value)
        res = await session.exec(select(column).distinct())
        groups[group.value] = [row for row in res.all()]

    return groups


@router.get("/map", response_model=list[LayerRead])
async def get_all_map_layers(
    session: AsyncSession = Depends(get_session),
    crop: str = Query(...),
    water_model: str | None = Query(...),
    climate_model: str | None = Query(...),
    scenario: str | None = Query(...),
    variable: str | None = Query(...),
    year: int | None = Query(...),
) -> list[LayerRead]:
    """Get all Layer data for the Drop4Crop map

    Does not include disabled layers (enabled=False)
    """

    query = select(Layer).where(Layer.enabled == True)

    if crop:
        query = query.where(Layer.crop == crop)
    if water_model:
        query = query.where(Layer.water_model == water_model)
    if climate_model:
        query = query.where(Layer.climate_model == climate_model)
    if scenario:
        query = query.where(Layer.scenario == scenario)
    if variable:
        query = query.where(Layer.variable == variable)
    if year:
        query = query.where(Layer.year == year)

    res = await session.exec(query)

    return res.all()


@router.get("/{layer_id}", response_model=LayerReadAuthenticated)
async def get_layer(
    obj: CRUD = Depends(get_one),
    user: User = Depends(require_admin),
) -> LayerReadAuthenticated:
    """Get a layer by id"""

    return obj


@router.get("", response_model=list[LayerReadAuthenticated])
async def get_all_layers(
    data: Any = Depends(get_all_authenticated),
    session: AsyncSession = Depends(get_session),
    total_count: int = Depends(get_count),
    user: User = Depends(require_admin),
) -> list[LayerReadAuthenticated]:
    """Get all Layer data

    If token and user is admin, return additional fields
    """

    return data


@router.post("", response_model=LayerReadAuthenticated)
async def create_layer(
    create_obj: LayerCreate,
    background_tasks: BackgroundTasks,
    session: AsyncSession = Depends(get_session),
    user: User = Depends(require_admin),
) -> LayerReadAuthenticated:
    """Creates a layer data record"""

    obj = await create_one(create_obj.model_dump(), session, background_tasks)

    return obj


@router.put("/batch", response_model=list[LayerReadAuthenticated])
async def update_many(
    layer_batch: LayerUpdateBatch,
    background_tasks: BackgroundTasks,
    session: AsyncSession = Depends(get_session),
    user: User = Depends(require_admin),
) -> list[LayerReadAuthenticated]:
    """Update plots from a list of PlotUpdate objects"""

    objs = []
    for id in layer_batch.ids:
        update_obj = LayerUpdate.model_validate(layer_batch.data)
        obj = await update_one(
            layer_id=id,
            layer_update=update_obj,
            session=session,
            background_tasks=background_tasks,
        )
        objs.append(obj)

    return objs


@router.put("/{layer_id}", response_model=LayerReadAuthenticated)
async def update_layer(
    updated_layer: Layer = Depends(update_one),
    user: User = Depends(require_admin),
) -> LayerReadAuthenticated:
    """Update a layer by id"""

    return updated_layer


# @router.delete("/batch", response_model=list[str])
# async def delete_batch(
#     ids: list[UUID],
#     session: AsyncSession = Depends(get_session),
# ) -> list[str]:
#     """Delete by a list of ids"""

#     for id in ids:
#         obj = await crud.get_model_by_id(model_id=id, session=session)
#         if obj:
#             await session.delete(obj)

#     await session.commit()

#     return [str(obj_id) for obj_id in ids]


@router.delete("/{layer_id}")
async def delete_layer(
    layer: LayerRead = Depends(get_one),
    session: AsyncSession = Depends(get_session),
    user: User = Depends(require_admin),
) -> str:
    """Delete a layer by id"""

    id = layer.id

    # First delete the coveragestore from geoserver
    # Then delete the layer from the database
    async with httpx.AsyncClient() as client:
        url = (
            f"{config.GEOSERVER_URL}/rest/workspaces/"
            f"{config.GEOSERVER_WORKSPACE}/coveragestores/{layer.layer_name}"
        )
        auth = (config.GEOSERVER_USER, config.GEOSERVER_PASSWORD)
        res = await client.delete(url, auth=auth, params={"recurse": "true"})

    if res.status_code > 299:
        raise HTTPException(
            status_code=res.status_code,
            detail=res.text,
        )

    await session.delete(layer)
    await session.commit()

    return id

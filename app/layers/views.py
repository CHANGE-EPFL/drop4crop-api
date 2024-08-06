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
from app.layers.services import (
    get_count,
    get_all_authenticated,
    get_one,
    create_one,
    update_one,
    delete_one,
)
from typing import Any
from sqlmodel import select
from app.config import config
from app.auth import require_admin, User
from app.layers.uploads.views import router as uploads_router
from uuid import UUID
import aioboto3
from fastapi.responses import JSONResponse
from rasterio.io import MemoryFile
import rasterio
from app.s3.services import get_s3

router = APIRouter()

router.include_router(uploads_router, prefix="/uploads", tags=["uploads"])


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
        res = await session.exec(
            select(column).where(Layer.enabled).distinct()
        )
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

    query = select(Layer).where(Layer.enabled)

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


@router.get("/{layer_id}/value")
async def get_pixel_value(
    layer_id: str,
    lat: float,
    lon: float,
    s3: aioboto3.Session = Depends(get_s3),
):

    try:
        s3_response = await s3.get_object(
            Bucket=config.S3_BUCKET_ID,
            Key=f"{config.S3_PREFIX}/{layer_id}.tif",
        )
        content = await s3_response["Body"].read()

        with MemoryFile(content) as memfile:
            with memfile.open() as dataset:
                # Convert lat/lon to dataset coordinates
                row, col = dataset.index(lon, lat)
                rows, cols = rasterio.transform.rowcol(
                    dataset.transform, [lon], [lat]
                )
                # Get the pixel value at coordinates
                value = dataset.read(1)[row, col]

                return JSONResponse(content={"value": value})

    except Exception as e:
        print(e)
        raise HTTPException(status_code=500, detail=str(e))


@router.get("/{layer_id}", response_model=LayerReadAuthenticated)
async def get_layer(
    obj: LayerRead = Depends(get_one),
    user: User = Depends(require_admin),
) -> LayerReadAuthenticated:
    """Get a layer by id"""

    return obj


@router.get("", response_model=list[LayerReadAuthenticated])
async def get_all_layers(
    data: Any = Depends(get_all_authenticated),
    total_count: int = Depends(get_count),
    user: User = Depends(require_admin),
) -> list[LayerReadAuthenticated]:
    """Get all Layer data"""

    return data


@router.post("", response_model=LayerReadAuthenticated)
async def create_layer(
    obj: LayerRead = Depends(create_one),
    user: User = Depends(require_admin),
) -> LayerReadAuthenticated:
    """Creates a layer data record"""

    return obj


@router.put("/batch", response_model=list[LayerReadAuthenticated])
async def update_many(
    layer_batch: LayerUpdateBatch,
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


@router.delete("/batch", response_model=list[UUID])
async def delete_batch(
    ids: list[UUID],
    background_tasks: BackgroundTasks,
    session: AsyncSession = Depends(get_session),
    s3: aioboto3.Session = Depends(get_s3),
) -> list[UUID]:
    """Delete by a list of ids"""

    deleted_ids = []
    for id in ids:
        background_tasks.add_task(delete_one, id, session, s3)
        deleted_ids.append(id)

    return deleted_ids


@router.delete("/{layer_id}")
async def delete_layer(
    layer: UUID = Depends(delete_one),
    user: User = Depends(require_admin),
) -> Any:
    """Delete a layer by id"""

    return layer
